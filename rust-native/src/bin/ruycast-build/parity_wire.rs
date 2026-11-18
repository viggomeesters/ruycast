//! Source mentions are candidates, never wire-behavior acceptance.
use super::*;
use serde_json::json;
const APP: &str = "Sources/RuycastApp/AppState.swift";
const NATIVE: &str = "rust-native/src/runtime.rs";

// Match Python str.splitlines(), including its less common Unicode separators.
fn lines(text: &str) -> Vec<&str> {
    let mut result = Vec::new();
    let mut start = 0;
    let mut chars = text.char_indices().peekable();
    while let Some((index, ch)) = chars.next() {
        if matches!(
            ch,
            '\n' | '\r' | '\u{b}' | '\u{c}' | '\u{1c}'
                ..='\u{1e}' | '\u{85}' | '\u{2028}' | '\u{2029}'
        ) {
            result.push(&text[start..index]);
            start = index + ch.len_utf8();
            if ch == '\r' && chars.peek().is_some_and(|(_, next)| *next == '\n') {
                let (index, _) = chars.next().unwrap();
                start = index + 1;
            }
        }
    }
    if start < text.len() {
        result.push(&text[start..]);
    }
    result
}
fn quoted_labels(text: &str) -> Vec<&str> {
    let mut result = Vec::new();
    let mut rest = text;
    while let Some(open) = rest.find('"') {
        rest = &rest[open + 1..];
        let Some(close) = rest.find('"') else { break };
        if close != 0 {
            result.push(&rest[..close]);
            rest = &rest[close + 1..];
        }
    }
    result
}
fn operations<'a>(app: &'a str, marker: &str) -> Result<Vec<&'a str>> {
    if marker.is_empty() {
        return Err("Empty wire handler marker.".into());
    }
    let start = app.find(marker).ok_or("Wire handler marker missing.")?;
    Ok(lines(&app[start..])
        .into_iter()
        .filter(|line| {
            line.trim_start_matches(|ch: char| {
                ch.is_whitespace() || matches!(ch, '\u{1c}'..='\u{1f}')
            })
            .starts_with("case \"")
        })
        .flat_map(|line| quoted_labels(line.split(':').next().unwrap()))
        .collect())
}
pub(super) fn run(root: &Path, marker: &str) -> Result<serde_json::Value> {
    let app_bytes = regular_bytes(&root.join(APP))?;
    let native_bytes = regular_bytes(&root.join(NATIVE))?;
    let app = std::str::from_utf8(&app_bytes).map_err(error)?;
    let native = std::str::from_utf8(&native_bytes).map_err(error)?;
    let native_lines = lines(native);
    let wire = operations(app, marker)?.into_iter().map(|op| {
        let needle = format!("\"{op}\"");
        let mentions = native_lines.iter().enumerate().filter_map(|(index, line)| line.contains(&needle).then_some(index + 1)).collect::<Vec<_>>();
        json!({"operation":op,"production":"Sources/RuycastApp/AppState.swift:handle(_:sink:)","nativeMentionLines":mentions,"acceptance":"Unproven full wire parity; a mention or fallback stub is not actual entry/effect/response evidence."})
    }).collect::<Vec<_>>();
    let sources = BTreeMap::from([
        (APP, format!("{:x}", Sha256::digest(&app_bytes))),
        (NATIVE, format!("{:x}", Sha256::digest(&native_bytes))),
    ]);
    for (path, sha) in &sources {
        if digest(&root.join(path))? != *sha {
            return Err("Wire inventory source changed.".into());
        }
    }
    Ok(json!({"wireOperations":wire,"sources":sources}))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn python_line_and_quote_semantics() {
        assert_eq!(
            lines("a\r\nb\rc\u{b}d\u{c}e\u{1c}f\u{1d}g\u{1e}h\u{85}i\u{2028}j\u{2029}"),
            vec!["a", "b", "c", "d", "e", "f", "g", "h", "i", "j"]
        );
        assert!(lines("").is_empty());
        assert_eq!(quoted_labels("\"a\", \"b\""), vec!["a", "b"]);
        assert_eq!(quoted_labels("\"\" \"b\""), vec![" "]);
        assert!(operations("data", "missing").is_err());
        assert!(operations("data", "").is_err());
    }
}
