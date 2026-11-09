//! Shared URL/snippet template semantics. No pasteboard access or execution.
//! Callers own interaction/origin and supply their clipboard/date context.
use objc2::{rc::autoreleasepool, AnyThread};
use objc2_foundation::{NSCharacterSet, NSDate, NSDateFormatter, NSLocale, NSString, NSTimeZone};
use rustcast_core::Command;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Field {
    pub id: String,
    pub label: String,
    pub default_value: String,
    pub options: Vec<String>,
}
/// Callers distinguish transport/fixture admission from application data.
#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Limits {
    pub source_bytes: usize,
    pub tokens: usize,
    pub output_bytes: usize,
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            source_bytes: 1_048_576,
            tokens: 16_384,
            output_bytes: 1_048_576,
        }
    }
}
impl Limits {
    /// The existing application imposes no template data/token quota. A socket
    /// packet limit does not bound saved configuration or expanded output.
    /// Retain checked arithmetic and Rust's addressable allocation boundary.
    pub fn application() -> Self {
        Self {
            source_bytes: isize::MAX as usize,
            tokens: isize::MAX as usize,
            output_bytes: isize::MAX as usize,
        }
    }
}
pub struct Context<'a> {
    pub clipboard: Option<&'a str>,
    pub now: f64,
    pub locale: Option<&'a str>,
    pub time_zone: Option<&'a str>,
}
impl<'a> Context<'a> {
    pub fn current(clipboard: Option<&'a str>) -> Self {
        Self {
            clipboard,
            now: NSDate::date().timeIntervalSince1970(),
            locale: None,
            time_zone: None,
        }
    }
}
struct Token {
    start: usize,
    end: usize,
    value: String,
}
pub struct Plan {
    source: String,
    url: bool,
    tokens: Vec<Token>,
    fields: Vec<Field>,
    limits: Limits,
}
/// AppState's Swift String.contains literal demand check, including Character
/// boundaries. Rendering itself intentionally uses the existing regex language.
pub fn needs_clipboard(source: &str) -> bool {
    autoreleasepool(|_| {
        let native = NSString::from_str(source);
        let mut scanned = 0;
        let mut utf16 = 0;
        source.match_indices("{clipboard}").any(|(byte, token)| {
            utf16 += source[scanned..byte].encode_utf16().count();
            let start = utf16;
            let end = start + token.len();
            scanned = byte + token.len();
            utf16 = end;
            let first = native.rangeOfComposedCharacterSequenceAtIndex(start);
            let last = native.rangeOfComposedCharacterSequenceAtIndex(end - 1);
            first.location == start && last.location + last.length == end
        })
    })
}
pub(crate) fn canonical(s: &str) -> String {
    NSString::from_str(s)
        .precomposedStringWithCanonicalMapping()
        .to_string()
}
fn trim(s: &str) -> String {
    NSString::from_str(s)
        .stringByTrimmingCharactersInSet(&NSCharacterSet::whitespaceAndNewlineCharacterSet())
        .to_string()
}
// Exact equivalent of the existing unanchored name="([^"]*)" regex.
// Deliberately retain substring matches such as xname= and empty attributes.
fn attribute<'a>(name: &str, token: &'a str) -> Option<&'a str> {
    let needle = format!("{name}=\"");
    let start = token.find(&needle)? + needle.len();
    let end = token[start..].find('"')? + start;
    Some(&token[start..end])
}
fn name(token: &str) -> &str {
    attribute("name", token).unwrap_or(if token == "query" || token == "argument" {
        "Query"
    } else {
        token
    })
}
fn automatic(token: &str) -> bool {
    ["clipboard", "cursor", "date", "time", "datetime", "day"].contains(&token)
        || has_prefix(token, "date ")
        || has_prefix(token, "datetime:")
}
fn has_prefix(value: &str, prefix: &str) -> bool {
    if !value.starts_with(prefix) {
        return false;
    }
    // Swift hasPrefix compares Characters: a combining mark after the final
    // space/colon can extend that Character and invalidate the ASCII prefix.
    // Both call-site prefixes are nonempty ASCII.
    let end = prefix.len();
    let native = NSString::from_str(value);
    let boundary = native.rangeOfComposedCharacterSequenceAtIndex(end - 1);
    boundary.location + boundary.length == end
}
fn append_option(
    piece: &str,
    output: &mut Vec<String>,
    seen: &mut std::collections::HashSet<String>,
) {
    let item = autoreleasepool(|_| {
        let item = trim(piece);
        (!item.is_empty()).then(|| {
            let key = canonical(&item);
            (item, key)
        })
    });
    if let Some((item, key)) = item {
        if seen.insert(key) {
            output.push(item);
        }
    }
}
// Swift split(separator: Character) does not split a comma carrying a
// combining mark. Use Foundation composed boundaries, not scalar splitting.
fn options(value: &str) -> Vec<String> {
    let native = NSString::from_str(value);
    let mut offset = 0;
    let mut start = 0;
    let mut output = vec![];
    let mut seen = std::collections::HashSet::new();
    for (byte, c) in value.char_indices() {
        if c == ',' {
            let range = native.rangeOfComposedCharacterSequenceAtIndex(offset);
            if range.location == offset && range.length == 1 {
                append_option(&value[start..byte], &mut output, &mut seen);
                start = byte + 1;
            }
        }
        offset += c.len_utf16();
    }
    append_option(&value[start..], &mut output, &mut seen);
    output
}
fn drop_characters(value: &str, count: usize) -> String {
    let value = NSString::from_str(value);
    let mut at = 0;
    for _ in 0..count {
        if at == value.length() {
            break;
        }
        let range = value.rangeOfComposedCharacterSequenceAtIndex(at);
        at = range.location + range.length;
    }
    value.substringFromIndex(at).to_string()
}
impl Plan {
    pub fn parse(command: &Command, limits: Limits) -> Result<Self, String> {
        if command
            .target
            .len()
            .checked_add(command.input_prompt.as_ref().map_or(0, String::len))
            .is_none_or(|n| n > limits.source_bytes)
        {
            return Err("Template source exceeds the supported size.".into());
        }
        autoreleasepool(|_| {
            // ASCII brace scanning is the same nonempty, nonnested language as
            // \{([^{}]+)\}; UTF-8 byte boundaries remain valid by construction.
            let mut tokens = vec![];
            let mut start = None;
            for (at, byte) in command.target.bytes().enumerate() {
                if byte == b'{' {
                    start = Some(at);
                } else if byte == b'}' {
                    if let Some(first) = start.take().filter(|s| at > s + 1) {
                        if tokens.len() >= limits.tokens {
                            return Err("Template has too many fields.".into());
                        }
                        tokens.push(Token {
                            start: first,
                            end: at + 1,
                            value: command.target[first + 1..at].to_string(),
                        });
                    }
                }
            }
            let mut fields = vec![];
            let mut seen = std::collections::HashSet::new();
            if let Some(prompt) = &command.input_prompt {
                fields.push(Field {
                    id: "input".into(),
                    label: prompt.clone(),
                    default_value: String::new(),
                    options: vec![],
                });
                seen.insert("input".into());
            }
            if command.kind == "url" || command.kind == "snippet" {
                for token in &tokens {
                    if automatic(&token.value) {
                        continue;
                    }
                    let id = name(&token.value);
                    if seen.insert(canonical(id)) {
                        fields.push(Field {
                            id: id.into(),
                            label: id.into(),
                            default_value: attribute("default", &token.value).unwrap_or("").into(),
                            options: options(attribute("options", &token.value).unwrap_or("")),
                        });
                    }
                }
            }
            Ok(Self {
                source: command.target.clone(),
                url: command.kind == "url",
                tokens,
                fields,
                limits,
            })
        })
    }
    pub fn fields(&self) -> &[Field] {
        &self.fields
    }
    pub fn needs_interaction(&self) -> bool {
        self.fields
            .iter()
            .any(|f| !f.options.is_empty() || f.default_value.is_empty())
    }
    /// Values use canonical Swift String equality. Later canonical duplicates
    /// replace earlier ones, like insertion into a Swift Dictionary. Callers
    /// must preserve their actual decoded dictionary ownership/order.
    pub fn render(
        &self,
        values: &[(String, String)],
        context: &Context<'_>,
    ) -> Result<(String, Option<usize>), String> {
        if !context.now.is_finite() {
            return Err("Invalid template date.".into());
        }
        let mut input_size = context.clipboard.map_or(0, str::len);
        for (key, value) in values {
            input_size = input_size
                .checked_add(key.len())
                .and_then(|n| n.checked_add(value.len()))
                .ok_or("Template values exceed the supported size.")?;
        }
        for value in [context.locale, context.time_zone].into_iter().flatten() {
            input_size = input_size
                .checked_add(value.len())
                .ok_or("Template values exceed the supported size.")?;
        }
        if input_size > self.limits.source_bytes {
            return Err("Template values exceed the supported size.".into());
        }
        autoreleasepool(|_| {
            let locale = context
                .locale
                .map(|s| {
                    NSLocale::initWithLocaleIdentifier(NSLocale::alloc(), &NSString::from_str(s))
                })
                .unwrap_or_else(NSLocale::currentLocale);
            let zone = match context.time_zone {
                Some(name) => NSTimeZone::timeZoneWithName(&NSString::from_str(name))
                    .ok_or("Invalid template time zone.")?,
                None => NSTimeZone::defaultTimeZone(),
            };
            let now = NSDate::dateWithTimeIntervalSince1970(context.now);
            let mut data = std::collections::HashMap::new();
            for (key, value) in values {
                data.insert(canonical(key), value.as_str());
            }
            let mut replacements = Vec::with_capacity(self.tokens.len());
            let mut size =
                self.source.len() - self.tokens.iter().map(|t| t.end - t.start).sum::<usize>();
            // Resolve right-to-left to preserve which existing validation error
            // wins. Build the final string only once, avoiding repeated suffix
            // shifts/allocation in the old right-to-left replacement loop.
            for token in self.tokens.iter().rev() {
                let t = token.value.as_str();
                let replacement = match t {
                    "cursor" => String::new(),
                    "clipboard" => context
                        .clipboard
                        .ok_or("Clipboard text is unavailable.")?
                        .to_string(),
                    _ if automatic(t) => {
                        let format = match t {
                            "date" => "yyyy-MM-dd".to_string(),
                            "time" => "HH:mm".into(),
                            "datetime" => "d MMMM yyyy HH:mm".into(),
                            "day" => "EEEE".into(),
                            _ => attribute("format", t)
                                .map(String::from)
                                .unwrap_or_else(|| drop_characters(t, "datetime:".len())),
                        };
                        let formatter = NSDateFormatter::new();
                        formatter.setLocale(Some(&locale));
                        formatter.setTimeZone(Some(&zone));
                        formatter.setDateFormat(Some(&NSString::from_str(&format)));
                        formatter.stringFromDate(&now).to_string()
                    }
                    _ => {
                        let field = name(t);
                        let value = data
                            .get(&canonical(field))
                            .copied()
                            .or_else(|| attribute("default", t))
                            .unwrap_or("");
                        if trim(value).is_empty() {
                            return Err(format!("Enter {field}."));
                        }
                        if self.url {
                            NSString::from_str(value)
                                .stringByAddingPercentEncodingWithAllowedCharacters(
                                    &NSCharacterSet::alphanumericCharacterSet(),
                                )
                                .map(|s| s.to_string())
                                .unwrap_or_default()
                        } else {
                            value.to_string()
                        }
                    }
                };
                size = size
                    .checked_add(replacement.len())
                    .filter(|n| *n <= self.limits.output_bytes)
                    .ok_or("Rendered template exceeds the supported size.")?;
                replacements.push(replacement);
            }
            if size > self.limits.output_bytes {
                return Err("Rendered template exceeds the supported size.".into());
            }
            let mut text = String::with_capacity(size);
            let mut cursor = None;
            let mut at = 0;
            let mut utf16 = 0;
            for (token, replacement) in self.tokens.iter().zip(replacements.into_iter().rev()) {
                let prefix = &self.source[at..token.start];
                text.push_str(prefix);
                utf16 += prefix.encode_utf16().count();
                if token.value == "cursor" && cursor.is_none() {
                    cursor = Some(utf16);
                }
                text.push_str(&replacement);
                utf16 += replacement.encode_utf16().count();
                at = token.end;
            }
            text.push_str(&self.source[at..]);
            Ok((text, cursor))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn command(text: &str) -> Command {
        serde_json::from_value(
            serde_json::json!({"id":"fixture","title":"Fixture","kind":"snippet","target":text}),
        )
        .unwrap()
    }
    fn context() -> Context<'static> {
        Context {
            clipboard: None,
            now: 0.,
            locale: Some("en_GB"),
            time_zone: Some("UTC"),
        }
    }
    #[test]
    fn nonfinite_date_and_oversized_values_fail_before_native_formatting() {
        let plan = Plan::parse(&command("{date}{x}"), Limits::default()).unwrap();
        let mut ctx = context();
        ctx.now = f64::NAN;
        assert_eq!(
            plan.render(&[], &ctx).unwrap_err(),
            "Invalid template date."
        );
        ctx.now = f64::INFINITY;
        assert_eq!(
            plan.render(&[], &ctx).unwrap_err(),
            "Invalid template date."
        );
        ctx.now = 0.;
        let values = vec![("x".into(), "a".repeat(1_048_577))];
        assert_eq!(
            plan.render(&values, &ctx).unwrap_err(),
            "Template values exceed the supported size."
        );
    }
    #[test]
    fn configurable_render_budget_and_leftmost_utf16_cursor_survive_expansion() {
        let plan = Plan::parse(
            &command("{x}{cursor}{x}{cursor}"),
            Limits {
                source_bytes: 100,
                tokens: 10,
                output_bytes: 12,
            },
        )
        .unwrap();
        assert_eq!(
            plan.render(&[("x".into(), "Ω😀".into())], &context())
                .unwrap(),
            ("Ω😀Ω😀".into(), Some(3))
        );
        assert!(plan
            .render(&[("x".into(), "long value".into())], &context())
            .is_err());
    }
    #[test]
    fn missing_input_uses_original_rightmost_error_priority() {
        let plan = Plan::parse(&command("{x}{clipboard}{y}"), Limits::default()).unwrap();
        assert_eq!(plan.render(&[], &context()).unwrap_err(), "Enter y.");
        assert_eq!(
            plan.render(&[("y".into(), "Y".into())], &context())
                .unwrap_err(),
            "Clipboard text is unavailable."
        );
    }
    #[test]
    fn prompt_bytes_are_part_of_the_explicit_parse_policy() {
        let mut source = command("plain");
        source.input_prompt = Some("x".repeat(20));
        assert!(Plan::parse(
            &source,
            Limits {
                source_bytes: 24,
                ..Limits::default()
            }
        )
        .is_err());
        assert!(Plan::parse(
            &source,
            Limits {
                source_bytes: 25,
                ..Limits::default()
            }
        )
        .is_ok());
    }
}
