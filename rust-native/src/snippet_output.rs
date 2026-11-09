//! Native snippet payloads. Editable source remains Markdown; only explicit
//! teams/html modes interpret links. No clipboard, activation or key effects.
use objc2::rc::autoreleasepool;
use objc2_foundation::{
    NSCharacterSet, NSMatchingOptions, NSRange, NSRegularExpression, NSRegularExpressionOptions,
    NSString, NSStringCompareOptions, NSURL,
};
use serde::Serialize;
use unicode_segmentation::UnicodeSegmentation;

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Range {
    pub location: usize,
    pub length: usize,
}
impl From<NSRange> for Range {
    fn from(value: NSRange) -> Self {
        Self {
            location: value.location,
            length: value.length,
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Link {
    pub range: Range,
    pub label_range: Range,
    pub label: String,
    pub url: String,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Payload {
    pub text: String,
    pub html: Option<String>,
    pub cursor: Option<isize>,
}
fn swift_string(value: &NSString) -> String {
    crate::native_text::repaired(value)
}

fn decode_label(value: &NSString) -> String {
    let repaired = NSString::from_str(&swift_string(value));
    repaired
        .stringByReplacingOccurrencesOfString_withString_options_range(
            &NSString::from_str(r"\\([\[\]\\])"),
            &NSString::from_str("$1"),
            NSStringCompareOptions::RegularExpressionSearch,
            NSRange::new(0, repaired.length()),
        )
        .to_string()
}
/// Count visible Characters in the actual suffix after UTF16 cursor mapping.
/// Callers choose the existing effect-specific cap (launcher/rich keyword:
/// 10000; plain keyword has a distinct Unicode insertion path).
pub fn cursor_steps(text: &str, cursor: usize) -> usize {
    autoreleasepool(|_| {
        let native = NSString::from_str(text);
        let suffix = swift_string(&native.substringFromIndex(cursor.min(native.length())));
        // NSString composed sequences differ from Swift Character for CRLF
        // and leading Indic joiners. Use extended graphemes on the repaired
        // actual suffix, verified against the current production Swift path.
        suffix.graphemes(true).count()
    })
}
pub fn valid_url(value: &str) -> bool {
    if value
        .graphemes(true)
        .any(|character| character.chars().next().is_some_and(char::is_whitespace))
    {
        return false;
    }
    autoreleasepool(|_| {
        let Some(url) = NSURL::URLWithString(&NSString::from_str(value)) else {
            return false;
        };
        let scheme = url
            .scheme()
            .map(|s| s.to_string().to_lowercase())
            .unwrap_or_default();
        match scheme.as_str() {
            "mailto" => {
                // Swift drops seven Characters, not seven Unicode scalars.
                value.graphemes(true).nth(7).is_some()
            }
            "http" | "https" => url.host().is_some_and(|host| !host.is_empty()),
            _ => false,
        }
    })
}
pub fn links(source: &str) -> Vec<Link> {
    autoreleasepool(|_| {
        // (A+|B)* and (A|B)* recognize the same URL text here: A is one
        // non-whitespace/non-parenthesis character; B is a balanced single
        // parenthesis group. Removing A's nested + avoids exponential failed
        // matches while retaining greedy match/capture boundaries.
        let pattern =
            NSString::from_str(r"\[((?:\\.|[^\]\r\n\\])*)\]\(((?:[^\s()]|\([^\s()]*\))*)\)");
        let Ok(regex) = NSRegularExpression::regularExpressionWithPattern_options_error(
            &pattern,
            NSRegularExpressionOptions::empty(),
        ) else {
            return vec![];
        };
        let source = NSString::from_str(source);
        regex
            .matchesInString_options_range(
                &source,
                NSMatchingOptions::empty(),
                NSRange::new(0, source.length()),
            )
            .iter()
            .filter_map(|m| {
                let url = source.substringWithRange(m.rangeAtIndex(2)).to_string();
                valid_url(&url).then(|| Link {
                    range: m.range().into(),
                    label_range: m.rangeAtIndex(1).into(),
                    label: decode_label(&source.substringWithRange(m.rangeAtIndex(1))),
                    url,
                })
            })
            .collect()
    })
}
pub fn link_source(label: &str, url: &str) -> String {
    let label = label
        .replace('\\', "\\\\")
        .replace('[', "\\[")
        .replace(']', "\\]");
    let url = url.replace('(', "%28").replace(')', "%29");
    format!("[{label}]({url})")
}

/// Selection and original source belong to this one open link form. A stale
/// form cannot replace a range in a subsequently changed snippet.
#[derive(Clone, Debug)]
pub struct LinkDraft {
    source: std::sync::Arc<str>,
    range: Range,
    pub label: String,
    pub url: String,
}
impl LinkDraft {
    pub fn range(&self) -> Range {
        self.range.clone()
    }
    pub fn new(source: &str, selection: Range) -> Self {
        if let Some(link) = links(source).into_iter().find(|link| {
            let end = link.range.location + link.range.length;
            (selection.length > 0
                && selection.location < end
                && selection.location.saturating_add(selection.length) > link.range.location)
                || (selection.length == 0
                    && selection.location >= link.range.location
                    && selection.location < end)
        }) {
            return Self {
                source: source.into(),
                range: link.range,
                label: link.label,
                url: link.url,
            };
        }
        let native = NSString::from_str(source);
        let location = selection.location.min(native.length());
        let length = selection.length.min(native.length() - location);
        let label = swift_string(&native.substringWithRange(NSRange::new(location, length)));
        Self {
            source: source.into(),
            range: Range { location, length },
            label,
            url: String::new(),
        }
    }
    pub fn apply(&self, current: &str) -> Option<(String, Range)> {
        if self.validation_error(current).is_some() {
            return None;
        }
        let replacement = link_source(&self.label, &self.url);
        let native = NSString::from_str(current);
        let updated = native.stringByReplacingCharactersInRange_withString(
            NSRange::new(self.range.location, self.range.length),
            &NSString::from_str(&replacement),
        );
        Some((
            swift_string(&updated),
            Range {
                location: self.range.location + replacement.encode_utf16().count(),
                length: 0,
            },
        ))
    }
    pub fn validation_error(&self, current: &str) -> Option<&'static str> {
        if current != self.source.as_ref() {
            return Some("Snippet text changed. Cancel and reopen the link.");
        }
        if self.label.is_empty() {
            return Some("Enter link text.");
        }
        if self.label.chars().any(|c| {
            matches!(
                c,
                '\n' | '\r' | '\u{b}' | '\u{c}' | '\u{85}' | '\u{2028}' | '\u{2029}'
            )
        }) {
            return Some("Link text must be one line.");
        }
        if !valid_url(&self.url) {
            return Some("Enter an http, https or mailto URL.");
        }
        None
    }
}
/// Match the original ruler: logical lines begin after LF UTF-16 units,
/// including a final empty line. Wrapped fragments keep the same line number.
pub fn line_starts_utf16(source: &str) -> Vec<usize> {
    std::iter::once(0)
        .chain(
            source
                .encode_utf16()
                .enumerate()
                .filter_map(|(i, unit)| (unit == 10).then_some(i + 1)),
        )
        .collect()
}
fn escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

pub fn render(source: &str, mode: Option<&str>, cursor: Option<isize>) -> Payload {
    if !matches!(mode, Some("teams" | "html")) {
        return Payload {
            text: source.into(),
            html: None,
            cursor,
        };
    }
    autoreleasepool(|_| {
        let ns = NSString::from_str(source);
        let mut plain = String::new();
        let mut rich = String::new();
        let mut position = 0usize;
        let mut mapped = None;
        let mut plain_utf16 = 0usize;
        for link in links(source) {
            let before = ns
                .substringWithRange(NSRange::new(position, link.range.location - position))
                .to_string();
            if let Some(cursor) = cursor
                .filter(|_| mapped.is_none())
                .filter(|c| *c <= link.range.location as isize)
            {
                mapped =
                    Some(plain_utf16 as isize + cursor.saturating_sub(position as isize).max(0));
            }
            plain_utf16 += before.encode_utf16().count();
            plain.push_str(&before);
            rich.push_str(&escape(&before));
            if let Some(cursor) = cursor
                .filter(|_| mapped.is_none())
                .filter(|c| *c < (link.range.location + link.range.length) as isize)
            {
                let raw = ns.substringWithRange(NSRange::new(
                    link.label_range.location,
                    link.label_range.length,
                ));
                let count = cursor
                    .saturating_sub(link.label_range.location as isize)
                    .clamp(0, raw.length() as isize) as usize;
                mapped = Some(
                    (plain_utf16
                        + decode_label(&raw.substringToIndex(count))
                            .encode_utf16()
                            .count()) as isize,
                );
            }
            plain_utf16 += link.label.encode_utf16().count();
            plain.push_str(&link.label);
            rich.push_str(&format!(
                "<a href=\"{}\">{}</a>",
                escape(&link.url),
                escape(&link.label)
            ));
            position = link.range.location + link.range.length;
        }
        if let Some(cursor) = cursor.filter(|_| mapped.is_none()) {
            mapped = Some(
                plain_utf16 as isize
                    + cursor
                        .min(ns.length() as isize)
                        .saturating_sub(position as isize)
                        .max(0),
            );
        }
        let tail = ns.substringFromIndex(position).to_string();
        plain.push_str(&tail);
        rich.push_str(&escape(&tail));
        let body = if mode == Some("teams") {
            rich.split('\n')
                .map(|line| {
                    let empty = NSString::from_str(line)
                        .stringByTrimmingCharactersInSet(
                            &NSCharacterSet::whitespaceAndNewlineCharacterSet(),
                        )
                        .is_empty();
                    format!(
                        "<p style=\"margin:0\">{}</p>",
                        if empty { "<br>" } else { line }
                    )
                })
                .collect::<String>()
        } else {
            format!(
                "<div>{}</div>",
                rich.split('\n').collect::<Vec<_>>().join("<br>")
            )
        };
        Payload {
            text: plain,
            html: Some(format!(
                "<html><head><meta charset=\"utf-8\"></head><body>{body}</body></html>"
            )),
            cursor: mapped,
        }
    })
}

#[cfg(test)]
mod link_editor_tests {
    use super::*;
    #[test]
    fn link_draft_preserves_utf16_selection_edit_and_source_authority() {
        let mut draft = LinkDraft::new(
            "😀 Guide tail",
            Range {
                location: 3,
                length: 5,
            },
        );
        assert_eq!(draft.label, "Guide");
        draft.label = "A [guide]".into();
        draft.url = "https://example.com/a(b)".into();
        let (source, caret) = draft.apply("😀 Guide tail").unwrap();
        assert_eq!(
            source,
            "😀 [A \\[guide\\]](https://example.com/a%28b%29) tail"
        );
        assert_eq!(caret.location, source.encode_utf16().count() - 5);
        assert_eq!(caret.length, 0);
        assert!(draft.apply("😀 Changed tail").is_none());
        let edit = LinkDraft::new(
            &source,
            Range {
                location: 5,
                length: 0,
            },
        );
        assert_eq!(edit.label, "A [guide]");
        assert_eq!(edit.url, "https://example.com/a%28b%29");
        assert_eq!(edit.apply(&source).unwrap().0, source);
        let clipped = LinkDraft::new(
            "x",
            Range {
                location: usize::MAX,
                length: usize::MAX,
            },
        );
        assert_eq!(
            clipped.range,
            Range {
                location: 1,
                length: 0
            }
        );
    }
    #[test]
    fn link_validation_and_logical_line_numbers_match_reference() {
        let mut draft = LinkDraft::new(
            "text",
            Range {
                location: 0,
                length: 4,
            },
        );
        for label in ["", "a\nb", "a\rb", "a\u{85}b", "a\u{2028}b", "a\u{2029}b"] {
            draft.label = label.into();
            draft.url = "https://example.com".into();
            assert!(draft.apply("text").is_none());
        }
        draft.label = "Guide".into();
        for url in [
            "",
            "file:///private",
            "javascript:alert(1)",
            "https://example.com/a b",
        ] {
            draft.url = url.into();
            assert!(draft.apply("text").is_none());
        }
        assert_eq!(line_starts_utf16(""), vec![0]);
        assert_eq!(line_starts_utf16("😀\r\nsecond\n"), vec![0, 4, 11]);
    }
}
