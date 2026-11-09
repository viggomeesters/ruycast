//! Ephemeral keyword semantics. No hooks, clipboard, logging or persistence.
//! Native target/generation/secure-field authority belongs to the future caller.
use crate::{
    snippet_output,
    templates::{self, Context, Limits, Plan},
};
use rustcast_core::Command;
use unicode_segmentation::UnicodeSegmentation;

struct Entry {
    command: Command,
    starts_word: bool,
}
pub struct Catalog {
    plans: std::collections::HashMap<String, Plan>,
    by_length: std::collections::BTreeMap<usize, std::collections::HashMap<String, Vec<Entry>>>,
}
impl Catalog {
    pub fn plan(&self, id: &str) -> Option<&Plan> {
        self.plans.get(id)
    }
    /// Prepare regex/template metadata on config adoption, never in a key tap.
    pub fn new(commands: &[Command]) -> Result<Self, String> {
        let mut by_length: std::collections::BTreeMap<
            usize,
            std::collections::HashMap<String, Vec<Entry>>,
        > = Default::default();
        let mut plans = std::collections::HashMap::new();
        for command in commands {
            if command.enabled == Some(false) || command.kind != "snippet" {
                continue;
            }
            let Some(keyword) = &command.keyword else {
                continue;
            };
            let plan = Plan::parse(command, Limits::application())?;
            if !plan
                .fields()
                .iter()
                .all(|f| !f.options.is_empty() || !f.default_value.is_empty())
            {
                continue;
            }
            plans.insert(command.id.clone(), plan);
            by_length
                .entry(keyword.graphemes(true).count())
                .or_default()
                .entry(templates::canonical(keyword))
                .or_default()
                .push(Entry {
                    command: command.clone(),
                    starts_word: word_character(keyword),
                });
        }
        Ok(Self { by_length, plans })
    }
}
/// Deliberately has no Debug/Serialize/buffer accessor: input never leaves here.
#[derive(Default)]
pub struct Matcher {
    buffer: String,
}
impl Matcher {
    pub fn reset(&mut self) {
        self.buffer = String::new();
    }
    pub fn backspace(&mut self) {
        if let Some((at, _)) = self.buffer.grapheme_indices(true).next_back() {
            self.buffer.truncate(at);
        }
    }
    pub fn type_text<'a>(&mut self, text: &str, catalog: &'a Catalog) -> Option<&'a Command> {
        if text.is_empty() {
            self.reset();
            return None;
        }
        self.buffer.push_str(text);
        if let Some((at, _)) = self.buffer.grapheme_indices(true).rev().nth(127) {
            if at != 0 {
                self.buffer = self.buffer[at..].to_owned();
            }
        }
        let boundaries: Vec<_> = self
            .buffer
            .grapheme_indices(true)
            .map(|(i, _)| i)
            .chain(std::iter::once(self.buffer.len()))
            .collect();
        let count = boundaries.len() - 1;
        for (&length, keywords) in catalog.by_length.range(..=count).rev() {
            let at = boundaries[count - length];
            let raw = &self.buffer[at..];
            let suffix = if raw.is_ascii() {
                std::borrow::Cow::Borrowed(raw)
            } else {
                std::borrow::Cow::Owned(templates::canonical(raw))
            };
            let Some(entries) = keywords.get(suffix.as_ref()) else {
                continue;
            };
            drop(suffix);
            let previous_word =
                count > length && word_character(&self.buffer[boundaries[count - length - 1]..at]);
            if let Some(entry) = entries.iter().find(|e| !e.starts_word || !previous_word) {
                self.reset();
                return Some(&entry.command);
            }
        }
        None
    }
}
// Swift's Character properties inspect the first scalar. Use the platform's
// Unicode binary/category properties for non-ASCII, including Apple's legacy
// private-use Alphabetic entries; Foundation's alphanumeric character set is
// a different property and omits them. The complete scalar union is compared
// to the actual shipped Swift implementation by the test-only oracle.
thread_local! {
    static WORD_CLASSIFIER: (objc2::rc::Retained<objc2_foundation::NSRegularExpression>,objc2::rc::Retained<objc2_foundation::NSRegularExpression>) = {
        let pattern = |s:&str|objc2_foundation::NSRegularExpression::regularExpressionWithPattern_options_error(
            &objc2_foundation::NSString::from_str(s),objc2_foundation::NSRegularExpressionOptions::empty())
            .expect("Static Unicode word property pattern");
        (pattern(r"\A\p{Alphabetic}"),pattern(r"\A[\p{Alphabetic}\p{Number}]"))
    };
}

/// Native host must call this on its actual callback thread before enabling
/// capture. Preparing a catalog on a worker cannot warm main's thread-local
/// native classifiers; never compile the patterns on a first intercepted key.
pub fn prepare_callback_thread() {
    objc2::rc::autoreleasepool(|_| WORD_CLASSIFIER.with(|_| {}));
}
pub fn word_character(character: &str) -> bool {
    let Some(scalar) = character.chars().next() else {
        return false;
    };
    if scalar.is_ascii() {
        return scalar.is_ascii_alphanumeric();
    }
    objc2::rc::autoreleasepool(|_| {
        WORD_CLASSIFIER.with(|patterns| {
            // The shipped Swift reference gives private-use scalars no numeric
            // value, even where Apple's ICU legacy category says Number.
            // Its Alphabetic property does retain the platform extension.
            let private =
                matches!(scalar as u32,0xe000..=0xf8ff|0xf0000..=0xffffd|0x100000..=0x10fffd);
            let regex = if private { &patterns.0 } else { &patterns.1 };
            let mut bytes = [0; 4];
            let source = objc2_foundation::NSString::from_str(scalar.encode_utf8(&mut bytes));
            regex
                .firstMatchInString_options_range(
                    &source,
                    objc2_foundation::NSMatchingOptions::empty(),
                    objc2_foundation::NSRange::new(0, source.length()),
                )
                .is_some()
        })
    })
}

pub struct Replacement {
    erase_count: usize,
    text: String,
    cursor: Option<usize>,
}
impl Replacement {
    pub fn new(keyword: &str, final_input: &str, text: String, cursor: Option<usize>) -> Self {
        Self {
            erase_count: keyword
                .graphemes(true)
                .count()
                .saturating_sub(final_input.graphemes(true).count()),
            text,
            cursor,
        }
    }
    pub fn erase_count(&self) -> usize {
        self.erase_count
    }
    pub fn text(&self) -> &str {
        &self.text
    }
    pub fn cursor(&self) -> Option<usize> {
        self.cursor
    }
    /// All steps are synchronous. The caller must validate target authority
    /// before applying and must not infer actual OS delivery from these calls.
    pub fn apply(&self, mut key: impl FnMut(u16), mut insert: impl FnMut(&str)) {
        self.try_apply(
            |k| {
                key(k);
                true
            },
            |text| {
                insert(text);
                true
            },
        );
    }
    /// Stop immediately at the first refused step; cancellation must not scan
    /// or allocate the remainder of a potentially large expanded output.
    pub fn try_apply(
        &self,
        mut key: impl FnMut(u16) -> bool,
        mut insert: impl FnMut(&str) -> bool,
    ) -> bool {
        for _ in 0..self.erase_count {
            if !key(51) {
                return false;
            }
        }
        let mut chunk = String::new();
        let mut units = 0;
        for scalar in self.text.chars() {
            if units + scalar.len_utf16() > 16 {
                if !insert(&chunk) {
                    return false;
                }
                chunk.clear();
                units = 0;
            }
            chunk.push(scalar);
            units += scalar.len_utf16();
        }
        if !chunk.is_empty() && !insert(&chunk) {
            return false;
        }
        if let Some(cursor) = self.cursor {
            for _ in 0..snippet_output::cursor_steps(&self.text, cursor) {
                if !key(123) {
                    return false;
                }
            }
        }
        true
    }
}
pub fn allows_expansion(secure_input: bool, subrole: Option<&str>) -> bool {
    !secure_input && subrole != Some("AXSecureTextField")
}
pub struct ChoiceRequest {
    command: Command,
    pid: i32,
    active: bool,
}
impl ChoiceRequest {
    pub fn new(command: Command, pid: i32) -> Self {
        Self {
            command,
            pid,
            active: true,
        }
    }
    pub fn command(&self) -> &Command {
        &self.command
    }
    pub fn pid(&self) -> i32 {
        self.pid
    }
    pub fn active(&self) -> bool {
        self.active
    }
    pub fn cancel(&mut self) {
        self.active = false;
    }
    pub fn replacement(
        &mut self,
        values: &[(String, String)],
        pid: i32,
        safe: bool,
        unchanged: bool,
        context: &Context<'_>,
    ) -> Result<Replacement, String> {
        if !self.active || self.pid != pid || !safe || !unchanged {
            return Err("The text field changed. Type the keyword again.".into());
        }
        let plan = Plan::parse(&self.command, Limits::application())?;
        let (text, cursor) = plan.render(values, context)?;
        self.active = false;
        Ok(Replacement::new(
            self.command.keyword.as_deref().unwrap_or(""),
            "",
            text,
            cursor,
        ))
    }
}

#[cfg(test)]
mod cancellation_checks {
    use super::*;
    use std::cell::Cell;
    #[test]
    fn first_refused_erase_skips_large_output_and_cursor() {
        let replacement = Replacement::new("abc", "c", "x".repeat(1_000_000), Some(0));
        let keys = Cell::new(0);
        let chunks = Cell::new(0);
        assert!(!replacement.try_apply(
            |_| {
                keys.set(keys.get() + 1);
                false
            },
            |_| {
                chunks.set(chunks.get() + 1);
                true
            }
        ));
        assert_eq!(keys.get(), 1);
        assert_eq!(chunks.get(), 0);
    }
    #[test]
    fn first_refused_chunk_stops_before_later_chunks_and_cursor() {
        let replacement = Replacement::new("x", "x", "😀".repeat(1_000_000), Some(0));
        let keys = Cell::new(0);
        let chunks = Cell::new(0);
        assert!(!replacement.try_apply(
            |_| {
                keys.set(keys.get() + 1);
                true
            },
            |s| {
                assert_eq!(s.encode_utf16().count(), 16);
                chunks.set(chunks.get() + 1);
                false
            }
        ));
        assert_eq!(keys.get(), 0);
        assert_eq!(chunks.get(), 1);
    }
}
