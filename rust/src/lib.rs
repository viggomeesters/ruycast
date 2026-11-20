//! Shared Rustcast domain logic. The Mac host is a native platform adapter.
pub mod script;
pub mod product_version;
use chrono::DateTime;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, HashSet},
    ffi::{CStr, CString},
    os::raw::c_char,
};

pub const LAYOUTS: &[&str] = &[
    "left-half",
    "right-half",
    "first-third",
    "center-third",
    "last-third",
    "first-two-thirds",
    "last-two-thirds",
    "maximize",
    "restore",
    "next-screen",
    "previous-screen",
];
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Command {
    pub id: String,
    pub title: String,
    pub kind: String,
    pub target: String,
    #[serde(default)]
    pub aliases: Vec<String>,
    #[serde(default)]
    pub arguments: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hotkey: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none", rename = "importedHotkey")]
    pub imported_hotkey: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout: Option<f64>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        rename = "inputPrompt"
    )]
    pub input_prompt: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub keyword: Option<String>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        rename = "applicationID"
    )]
    pub application_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        rename = "snippetOutput"
    )]
    pub snippet_output: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ClipboardSettings {
    pub enabled: bool,
    pub limit: usize,
    pub max_text_bytes: usize,
    pub retention_days: u32,
    pub excluded_apps: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_asset_bytes: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_total_asset_bytes: Option<usize>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AutoQuitSettings {
    #[serde(default, skip_serializing_if = "auto_quit_risk_is_false")]
    pub allow_unverified_activity: bool,
    #[serde(default)]
    pub enabled: bool,
    pub minutes: u32,
}
fn auto_quit_risk_is_false(value: &bool) -> bool { !*value }
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Config {
    pub version: u32,
    pub launcher_hotkey: String,
    pub clipboard_hotkey: String,
    pub clipboard: ClipboardSettings,
    #[serde(default)]
    pub snippet_expansion_enabled: bool,
    #[serde(default)]
    pub hyper_key_enabled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub separate_command_keys: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub theme: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub menu_bar_visible: Option<bool>,
    /// Bundle identifier -> preference, independent of launcher command IDs.
    /// Missing entries are disabled. This configuration alone never quits apps.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub auto_quit: BTreeMap<String, AutoQuitSettings>,
    pub commands: Vec<Command>,
}
pub fn default_config() -> Value {
    serde_json::from_str(include_str!("../../config/default.json")).expect("shipped default config")
}
pub fn shortcut(s: &str) -> Result<String, String> {
    let lower = s.to_lowercase();
    if lower == "fn" {
        return Ok(lower);
    }
    if let Some(key) = lower.strip_prefix("double:") {
        if ["ctrl", "alt", "cmd", "shift", "left-ctrl", "right-ctrl", "left-alt",
            "right-alt", "left-cmd", "right-cmd", "left-shift", "right-shift", "fn"].contains(&key)
            || (!key.contains(['+', ':'])
                && !["left-control", "left-option", "backspace"].contains(&key)
                && shortcut(&format!("ctrl+{key}")).is_ok())
        {
            return Ok(lower);
        }
        return Err("Choose a single supported key after double:.".into());
    }
    let mut parts = lower.split('+').collect::<Vec<_>>();
    let key = parts.pop().unwrap_or("");
    let known_key = key.len() == 1
        && (key.as_bytes()[0].is_ascii_alphanumeric() || "[],./-=\\';`".contains(key))
        || [
            "space",
            "tab",
            "return",
            "left",
            "right",
            "up",
            "down",
            "left-control",
            "left-option",
            "delete",
            "backspace",
            "escape",
            "f1",
            "f2",
            "f3",
            "f4",
            "f5",
            "f6",
            "f7",
            "f8",
            "f9",
            "f10",
            "f11",
            "f12",
        ]
        .contains(&key);
    let mut mods = HashSet::new();
    if !known_key
        || parts.is_empty()
        || parts.iter().any(|m| {
            !["ctrl", "alt", "right-alt", "shift", "cmd", "left-cmd", "right-cmd", "fn"].contains(m)
                || !mods.insert(*m)
        })
    {
        return Err(format!("Invalid hotkey: {s}. Use e.g. ctrl+alt+space."));
    }
    if ["cmd", "left-cmd", "right-cmd"]
        .iter()
        .filter(|m| mods.contains(**m))
        .count()
        > 1
    {
        return Err("Choose cmd, left-cmd or right-cmd, not a combination.".into());
    }
    if mods.contains("alt") && mods.contains("right-alt") {
        return Err("Choose alt or right-alt, not both.".into());
    }
    if (key == "left-control" && parts != ["ctrl"])
        || (key == "left-option" && parts != ["alt"])
    {
        return Err("Use Ctrl+Left-Control or Alt+Left-Option for a single modifier tap.".into());
    }
    if mods.contains("fn") && key.strip_prefix('f').is_some_and(|number| number.parse::<u8>().is_ok()) {
        return Err("Use Fn with an ordinary key. macOS does not consistently distinguish Fn with an F-key.".into());
    }
    parts.sort();
    Ok(format!("{}+{key}", parts.join("+")))
}
pub fn validate(value: &Value) -> Result<Config, String> {
    let c: Config = serde_json::from_value(value.clone()).map_err(|e| e.to_string())?;
    if c.version != 1 {
        return Err("Unsupported config version; expected 1.".into());
    }
    let mut auto_quit_apps = HashSet::new();
    for (application, preference) in &c.auto_quit {
        if application.split('.').any(|part| part.is_empty())
            || !application
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b".-".contains(&byte))
            || !auto_quit_apps.insert(application.to_ascii_lowercase())
        {
            return Err("Auto Quit requires unique application bundle identifiers (letters, digits, hyphens and periods).".into());
        }
        if preference.minutes == 0 {
            return Err("Auto Quit minutes must be a positive whole number.".into());
        }
    }
    if !(1..=1000).contains(&c.clipboard.limit)
        || !(1..=1_000_000).contains(&c.clipboard.max_text_bytes)
        || !(1..=365).contains(&c.clipboard.retention_days)
    {
        return Err(
            "Clipboard bounds: limit 1–1000, maxTextBytes 1–1000000, retentionDays 1–365.".into(),
        );
    }
    if c.clipboard
        .max_asset_bytes
        .is_some_and(|n| n == 0 || n > 100_000_000)
        || c.clipboard
            .max_total_asset_bytes
            .is_some_and(|n| n == 0 || n > 1_000_000_000)
    {
        return Err("Clipboard image limits must be positive, maxAssetBytes <=100MB and maxTotalAssetBytes <=1GB.".into());
    }
    if c.theme
        .as_ref()
        .is_some_and(|theme| !["system", "light", "dark", "graphite"].contains(&theme.as_str()))
    {
        return Err("Theme must be system, light, dark or graphite.".into());
    }
    let mut keys = HashSet::new();
    let mut names = HashSet::new();
    let mut keywords = HashSet::new();
    for k in [&c.launcher_hotkey, &c.clipboard_hotkey] {
        if !keys.insert(shortcut(k)?) {
            return Err(format!("Duplicate hotkey: {k}"));
        }
    }
    for cmd in &c.commands {
        if cmd.title.trim().is_empty() || cmd.target.is_empty() {
            return Err("Commands need id, title and target.".into());
        }
        for n in std::iter::once(&cmd.id).chain(cmd.aliases.iter()) {
            if n.trim().is_empty() || n.trim() != n || !names.insert(n.to_lowercase()) {
                return Err(format!("Empty or duplicate command id/alias: {n}"));
            }
        }
        if let Some(output) = &cmd.snippet_output {
            if cmd.kind != "snippet" || !["plain", "teams", "html"].contains(&output.as_str()) {
                return Err("Snippet output must be plain, teams or html on a snippet.".into());
            }
        }
        if let Some(keyword) = &cmd.keyword {
            if cmd.kind != "snippet" || !keywords.insert(keyword.clone()) {
                return Err("Duplicate or non-snippet keyword.".into());
            }
        }
        if let Some(k) = &cmd.hotkey {
            let canonical = shortcut(k)?;
            if cmd.enabled != Some(false) && !keys.insert(canonical) {
                return Err(format!("Duplicate hotkey: {k}"));
            }
        }
        if let Some(k) = &cmd.imported_hotkey {
            shortcut(k)?;
        }
        match cmd.kind.as_str() {
            "app" => {}
            "snippet" => {
                if let Some(keyword) = &cmd.keyword {
                    if keyword.is_empty()
                        || keyword.chars().any(char::is_whitespace)
                        || keyword.chars().count() > 64
                    {
                        return Err(
                            "Snippet keywords must be 1–64 characters without spaces.".into()
                        );
                    }
                }
            }
            "url" => {
                let scheme = cmd.target.split(':').next().unwrap_or("").to_lowercase();
                if !scheme
                    .chars()
                    .next()
                    .map(|c| c.is_ascii_alphabetic())
                    .unwrap_or(false)
                    || !scheme
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || "+.-".contains(c))
                    || ["javascript", "data"].contains(&scheme.as_str())
                    || !cmd.target.contains(':')
                {
                    return Err(format!("Unsupported URL in {}", cmd.id));
                }
            }
            "path" => {
                if !cmd.target.starts_with('/') && !cmd.target.starts_with("~/") {
                    return Err("Paths must be absolute or start with ~/".into());
                }
            }
            "script" => {
                if (!cmd.target.starts_with('/') && !cmd.target.starts_with("~/"))
                    || !(0.1..=300.).contains(&cmd.timeout.unwrap_or(30.))
                {
                    return Err(
                        "Script needs an absolute executable path and timeout 0.1–300 seconds."
                            .into(),
                    );
                }
            }
            "window" => {
                if !LAYOUTS.contains(&cmd.target.as_str()) {
                    return Err(format!("Unknown window layout: {}", cmd.target));
                }
            }
            "builtin" => {
                if ![
                    "clipboard",
                    "snippets",
                    "quicklinks",
                    "create-snippet",
                    "create-quicklink",
                    "settings",
                    "open-config",
                    "reload-config",
                    "analytics",
                    "search-files",
                    "search-menu-items",
                    "switch-windows",
                    "search-safari-tabs",
                    "apple-shortcuts",
                    "kill-process",
                    "system-monitor",
                    "lock-screen",
                    "utility-set-text",
                    "utility-paste-text",
                    "utility-text-history",
                    "utility-password",
                    "utility-username",
                    "utility-password-history",
                    "utility-username-history",
                ]
                .contains(&cmd.target.as_str())
                {
                    return Err(format!("Unknown built-in: {}", cmd.target));
                }
            }
            _ => return Err(format!("Unknown command kind: {}", cmd.kind)),
        }
    }
    for (single, doubles) in [
        ("ctrl+left-control", ["double:ctrl", "double:left-ctrl"]),
        ("alt+left-option", ["double:alt", "double:left-alt"]),
        ("fn", ["double:fn", "double:fn"]),
    ] {
        if keys.contains(single) && doubles.iter().any(|key| keys.contains(*key)) {
            return Err(format!("Choose a single or double tap for {single}."));
        }
    }
    Ok(c)
}
/// Immutable normalized catalog. Rebuilt only when commands change.
pub struct SearchIndex {
    entries: Vec<SearchEntry>,
}
struct SearchEntry {
    id: String,
    title: String,
    aliases: Vec<String>,
    haystack: String,
    words: Vec<String>,
    initials: String,
    title_order: usize,
}
impl SearchIndex {
    pub fn new(commands: &[Command]) -> Self {
        // Resolve stable title/catalog order once at the immutable catalog boundary.
        let mut result = Self {
            entries: commands
                .iter()
                .map(|c| SearchEntry {
                    title_order: 0,
                    words: c
                        .title
                        .to_lowercase()
                        .split(|c: char| !c.is_alphanumeric())
                        .filter(|w| !w.is_empty())
                        .map(str::to_owned)
                        .collect(),
                    initials: c
                        .title
                        .to_lowercase()
                        .split(|c: char| !c.is_alphanumeric())
                        .filter_map(|w| w.chars().next())
                        .collect(),
                    id: c.id.to_lowercase(),
                    title: c.title.to_lowercase(),
                    aliases: c
                        .aliases
                        .iter()
                        .chain(c.keyword.iter())
                        .map(|a| a.to_lowercase())
                        .collect(),
                    haystack: format!(
                        "{} {} {} {} {}",
                        c.title,
                        c.id,
                        if c.kind == "snippet" { "" } else { &c.target },
                        c.aliases.join(" "),
                        c.keyword.as_deref().unwrap_or("")
                    )
                    .to_lowercase(),
                })
                .collect(),
        };
        let mut order = (0..result.entries.len()).collect::<Vec<_>>();
        order.sort_by(|a, b| result.entries[*a].title.cmp(&result.entries[*b].title));
        for (rank, index) in order.into_iter().enumerate() {
            result.entries[index].title_order = rank;
        }
        result
    }
    pub fn search(&self, query: &str) -> Vec<usize> {
        self.search_ranked(query, &[])
    }
    /// Typed in-process entry for the Rust host. The C ABI remains available
    /// for the migration reference; invalid snapshots never reach the scorer.
    pub fn search_with_recency(&self, query: &str, recency: &[f64]) -> Result<Vec<usize>, String> {
        if !recency.is_empty()
            && (recency.len() != self.entries.len()
                || recency.iter().any(|v| !v.is_finite() || *v < 0.0))
        {
            return Err("Invalid catalog recency snapshot".into());
        }
        Ok(self.search_ranked(query, recency))
    }
    fn search_ranked(&self, query: &str, recency: &[f64]) -> Vec<usize> {
        let q = query.trim().to_lowercase();
        if q.is_empty() {
            return (0..self.entries.len()).collect();
        }
        let terms = q.split_whitespace().collect::<Vec<_>>();
        let fuzzy = q.chars().nth(1).is_some();
        let mut found = self
            .entries
            .iter()
            .enumerate()
            .filter_map(|(i, e)| {
                let score = if e.id == q || e.aliases.contains(&q) {
                    0
                } else if e.title == q {
                    1
                } else if e.title.starts_with(&q) {
                    2
                } else if terms
                    .iter()
                    .all(|w| e.words.iter().any(|word| word.starts_with(w)))
                    || (fuzzy && e.initials.starts_with(&q))
                {
                    3
                } else if terms.iter().all(|w| e.haystack.contains(w)) {
                    4
                } else if fuzzy && {
                    let mut title = e.title.chars();
                    q.chars()
                        .filter(|c| !c.is_whitespace())
                        .all(|needle| title.any(|c| c == needle))
                } {
                    5
                } else {
                    return None;
                };
                Some((score, i))
            })
            .collect::<Vec<_>>();
        // Exact/prefix/literal matches keep the full established result list.
        // A weak subsequence must not hide a direct title-word typo.
        if found.iter().all(|(score, _)| *score == 5)
            && (4..=64).contains(&q.chars().take(65).count())
            && q.chars().all(char::is_alphanumeric)
        {
            let had_weak = !found.is_empty();
            found.clear();
            found.extend(self.entries.iter().enumerate().filter_map(|(i, e)| {
                // This length check identifies a complete single-word title
                // without re-scanning long strings or storing more index data.
                let whole = e.words.len() == 1 && e.words[0].len() == e.title.len();
                let score = if whole && one_typo(&q, &e.title) {
                    5
                } else if !whole && e.words.iter().any(|word| one_typo(&q, word)) {
                    6
                } else if had_weak && {
                    let mut title = e.title.chars();
                    q.chars().all(|needle| title.any(|c| c == needle))
                } {
                    7
                } else {
                    return None;
                };
                Some((score, i))
            }));
        }
        found.sort_by(|a, b| {
            a.0.cmp(&b.0)
                .then_with(|| {
                    recency
                        .get(b.1)
                        .unwrap_or(&0.0)
                        .total_cmp(recency.get(a.1).unwrap_or(&0.0))
                })
                .then_with(|| {
                    self.entries[a.1]
                        .title_order
                        .cmp(&self.entries[b.1].title_order)
                })
        });
        found.into_iter().map(|(_, i)| i).collect()
    }
}
// One substitution, insertion, deletion or adjacent transposition, in Unicode
// scalars. Iterator comparisons avoid a dynamic-programming matrix/allocation.
fn one_typo(a: &str, b: &str) -> bool {
    let mut a = a.chars();
    let mut b = b.chars();
    loop {
        match (a.next(), b.next()) {
            (None, None) => return true,
            (None, Some(_)) => return b.next().is_none(),
            (Some(_), None) => return a.next().is_none(),
            (Some(x), Some(y)) if x == y => {}
            (Some(x), Some(y)) => {
                if a.clone().eq(b.clone())
                    || a.clone().eq(std::iter::once(y).chain(b.clone()))
                    || std::iter::once(x).chain(a.clone()).eq(b.clone())
                {
                    return true;
                }
                return a.next() == Some(y) && b.next() == Some(x) && a.eq(b);
            }
        }
    }
}
pub fn search(commands: &[Command], query: &str) -> Vec<Command> {
    SearchIndex::new(commands)
        .search(query)
        .into_iter()
        .map(|i| commands[i].clone())
        .collect()
}
/// Version of the statically linked C ABI in ruycast_core.h.
#[no_mangle]
pub extern "C" fn rustcast_abi_version() -> u32 {
    1
}

/// # Safety
/// `catalog` must be a live NUL-terminated UTF-8 JSON array for this call.
/// The returned immutable index must be freed exactly once with rustcast_index_free.
#[no_mangle]
pub unsafe extern "C" fn rustcast_index_new(catalog: *const c_char) -> *mut SearchIndex {
    std::panic::catch_unwind(|| {
        if catalog.is_null() {
            return std::ptr::null_mut();
        }
        match serde_json::from_slice::<Vec<Command>>(CStr::from_ptr(catalog).to_bytes()) {
            Ok(commands) => Box::into_raw(Box::new(SearchIndex::new(&commands))),
            Err(_) => std::ptr::null_mut(),
        }
    })
    .unwrap_or(std::ptr::null_mut())
}
/// # Safety
/// Index must be live; query is `length` readable UTF-8 bytes. Output is an owned
/// JSON array of indices, released with rustcast_free. Byte length preserves NULs.
#[no_mangle]
pub unsafe extern "C" fn rustcast_index_search(
    index: *const SearchIndex,
    query: *const u8,
    length: usize,
) -> *mut c_char {
    std::panic::catch_unwind(|| {
        if index.is_null() || query.is_null() {
            return std::ptr::null_mut();
        }
        let Ok(q) = std::str::from_utf8(std::slice::from_raw_parts(query, length)) else {
            return std::ptr::null_mut();
        };
        CString::new(json!((*index).search(q)).to_string())
            .unwrap()
            .into_raw()
    })
    .unwrap_or(std::ptr::null_mut())
}
/// # Safety
/// Index must be live; query is `length` readable UTF-8 bytes. Output, when
/// capacity > 0, is writable for `capacity` usize values and does not alias inputs.
/// Writes at most capacity indices, returns written count, or -1 for invalid input.
/// Swift Int and Rust usize have the same native word layout on supported macOS.
#[no_mangle]
pub unsafe extern "C" fn rustcast_index_search_into(
    index: *const SearchIndex,
    query: *const u8,
    length: usize,
    output: *mut usize,
    capacity: usize,
) -> isize {
    rustcast_index_search_ranked_into(index, query, length, std::ptr::null(), 0, output, capacity)
}
/// # Safety
/// Same requirements as rustcast_index_search_into; recency is either omitted
/// (count 0) or a readable finite nonnegative f64 weight per catalog entry.
#[no_mangle]
pub unsafe extern "C" fn rustcast_index_search_ranked_into(
    index: *const SearchIndex,
    query: *const u8,
    length: usize,
    recency: *const f64,
    recency_count: usize,
    output: *mut usize,
    capacity: usize,
) -> isize {
    std::panic::catch_unwind(|| {
        if index.is_null() || query.is_null() || (capacity > 0 && output.is_null()) {
            return -1;
        }
        let Ok(q) = std::str::from_utf8(std::slice::from_raw_parts(query, length)) else {
            return -1;
        };
        let weights = if recency_count == 0 {
            &[][..]
        } else {
            if recency.is_null() || recency_count != (*index).entries.len() {
                return -1;
            }
            let weights = std::slice::from_raw_parts(recency, recency_count);
            if weights.iter().any(|v| !v.is_finite() || *v < 0.0) {
                return -1;
            }
            weights
        };
        let found = (*index).search_ranked(q, weights);
        let count = found.len().min(capacity);
        if count > 0 {
            std::ptr::copy_nonoverlapping(found.as_ptr(), output, count);
        }
        count as isize
    })
    .unwrap_or(-1)
}
/// # Safety
/// Must receive an unfreed pointer from rustcast_index_new, or null.
#[no_mangle]
pub unsafe extern "C" fn rustcast_index_free(index: *mut SearchIndex) {
    if !index.is_null() {
        drop(Box::from_raw(index))
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Clip {
    pub id: String,
    pub text: String,
    pub date: String,
    pub source: String,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        rename = "contentType"
    )]
    pub content_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none", rename = "dataFile")]
    pub data_file: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none", rename = "fileURLs")]
    pub file_urls: Option<Vec<String>>,
}
pub fn history(
    items: &[Clip],
    clip: Option<Clip>,
    settings: &ClipboardSettings,
    now: &str,
) -> Result<Vec<Clip>, String> {
    let now = DateTime::parse_from_rfc3339(now)
        .map_err(|e| e.to_string())?
        .timestamp();
    let mut kept = items
        .iter()
        .filter(|c| {
            c.text.len() <= settings.max_text_bytes
                && DateTime::parse_from_rfc3339(&c.date)
                    .map(|d| now - d.timestamp() < i64::from(settings.retention_days) * 86400)
                    .unwrap_or(false)
        })
        .cloned()
        .collect::<Vec<_>>();
    if let Some(c) = clip {
        if settings.enabled
            && !c.text.trim().is_empty()
            && c.text.len() <= settings.max_text_bytes
            && !settings.excluded_apps.contains(&c.source)
        {
            kept.retain(|x| {
                x.id != c.id
                    && !(x.text == c.text
                        && x.content_type == c.content_type
                        && x.data_file == c.data_file
                        && x.file_urls == c.file_urls)
            });
            kept.insert(0, c)
        }
    }
    kept.truncate(settings.limit);
    Ok(kept)
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct Frame {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}
pub fn layout(f: Frame, name: &str) -> Result<Frame, String> {
    let (x, w) = match name {
        "left-half" => (0., 0.5),
        "right-half" => (0.5, 0.5),
        "first-third" => (0., 1. / 3.),
        "center-third" => (1. / 3., 1. / 3.),
        "last-third" => (2. / 3., 1. / 3.),
        "first-two-thirds" => (0., 2. / 3.),
        "last-two-thirds" => (1. / 3., 2. / 3.),
        "maximize" => (0., 1.),
        _ => return Err(format!("Unknown layout: {name}")),
    };
    Ok(Frame {
        x: f.x + f.width * x,
        width: f.width * w,
        ..f
    })
}
pub fn dispatch(op: &str, v: Value) -> Result<Value, String> {
    match op {
        "defaults" => Ok(default_config()),
        "validate" => serde_json::to_value(validate(&v)?).map_err(|e| e.to_string()),
        "shortcut" => Ok(json!(shortcut(
            v.as_str().ok_or("Expected a hotkey string.")?
        )?)),
        "search" => {
            let items: Vec<Command> =
                serde_json::from_value(v["commands"].clone()).map_err(|e| e.to_string())?;
            Ok(json!(search(&items, v["query"].as_str().unwrap_or(""))))
        }
        "history" => {
            let items = serde_json::from_value::<Vec<Clip>>(v["items"].clone())
                .map_err(|e| e.to_string())?;
            let s = serde_json::from_value::<ClipboardSettings>(v["settings"].clone())
                .map_err(|e| e.to_string())?;
            let clip = serde_json::from_value::<Option<Clip>>(v["clip"].clone())
                .map_err(|e| e.to_string())?;
            Ok(json!(history(
                &items,
                clip,
                &s,
                v["now"].as_str().ok_or("Missing now")?
            )?))
        }
        "layout" => Ok(json!(layout(
            serde_json::from_value(v["frame"].clone()).map_err(|e| e.to_string())?,
            v["name"].as_str().ok_or("Missing layout")?
        )?)),
        _ => Err(format!("Unknown core operation: {op}")),
    }
}
/// Synchronous C ABI. Inputs are borrowed UTF-8 C strings; caller frees the result.
/// # Safety
/// Non-null pointers must be NUL-terminated strings valid for this call.
/// Null arguments return an owned JSON error response.
#[no_mangle]
pub unsafe extern "C" fn rustcast_call(op: *const c_char, payload: *const c_char) -> *mut c_char {
    let response = std::panic::catch_unwind(|| {
        if op.is_null() || payload.is_null() {
            return Err("Null core argument.".into());
        }
        let op = CStr::from_ptr(op).to_str().map_err(|e| e.to_string())?;
        let v = serde_json::from_slice(CStr::from_ptr(payload).to_bytes())
            .map_err(|e| e.to_string())?;
        dispatch(op, v)
    })
    .unwrap_or_else(|_| Err("Rust core panicked.".into()));
    let value = match response {
        Ok(v) => json!({"ok":true,"result":v}),
        Err(e) => json!({"ok":false,"error":e}),
    };
    CString::new(value.to_string())
        .expect("JSON escapes NUL")
        .into_raw()
}
/// # Safety
/// Pointer must be an unfreed result returned by rustcast_call.
#[no_mangle]
pub unsafe extern "C" fn rustcast_free(ptr: *mut c_char) {
    if !ptr.is_null() {
        drop(CString::from_raw(ptr))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    struct IndexGuard(*mut SearchIndex);
    impl Drop for IndexGuard {
        fn drop(&mut self) {
            unsafe { rustcast_index_free(self.0) }
        }
    }

    #[test]
    fn abi_null_call_returns_owned_error_and_null_free_is_safe() {
        let response = unsafe { rustcast_call(std::ptr::null(), std::ptr::null()) };
        assert!(!response.is_null());
        let bytes = unsafe { CStr::from_ptr(response).to_bytes() };
        let value: Value = serde_json::from_slice(bytes).unwrap();
        assert_eq!(value["ok"], false);
        unsafe {
            rustcast_free(response);
            rustcast_free(std::ptr::null_mut());
            rustcast_index_free(std::ptr::null_mut());
        }
    }

    #[test]
    fn abi_index_search_preserves_capacity_canary_and_error_contract() {
        let commands: Vec<Command> = serde_json::from_value(serde_json::json!([
            { "id":"first", "title":"Same", "kind":"url", "target":"https://example.com" },
            { "id":"second", "title":"Same", "kind":"url", "target":"https://example.com" }
        ])).unwrap();
        let catalog = CString::new(serde_json::to_vec(&commands).unwrap()).unwrap();
        let index = IndexGuard(unsafe { rustcast_index_new(catalog.as_ptr()) });
        assert!(!index.0.is_null());

        let query = b"same";
        let mut output = [777usize; 4];
        let count = unsafe {
            rustcast_index_search_ranked_into(
                index.0,
                query.as_ptr(),
                query.len(),
                std::ptr::null(),
                0,
                output.as_mut_ptr().add(1),
                1,
            )
        };
        assert_eq!(count, 1);
        assert_eq!(output, [777, 0, 777, 777]);

        let invalid = [0xff];
        unsafe {
            assert_eq!(
                rustcast_index_search_ranked_into(
                    index.0,
                    invalid.as_ptr(),
                    1,
                    std::ptr::null(),
                    0,
                    std::ptr::null_mut(),
                    0
                ),
                -1
            );
            assert_eq!(
                rustcast_index_search_ranked_into(
                    std::ptr::null(),
                    query.as_ptr(),
                    query.len(),
                    std::ptr::null(),
                    0,
                    std::ptr::null_mut(),
                    0
                ),
                -1
            );
            assert_eq!(
                rustcast_index_search_ranked_into(
                    index.0,
                    std::ptr::null(),
                    1,
                    std::ptr::null(),
                    0,
                    std::ptr::null_mut(),
                    0
                ),
                -1
            );
            let weights = [-1.0, 0.0];
            assert_eq!(
                rustcast_index_search_ranked_into(
                    index.0,
                    query.as_ptr(),
                    query.len(),
                    weights.as_ptr(),
                    weights.len(),
                    std::ptr::null_mut(),
                    0
                ),
                -1
            );
        }
    }

    #[test]
    fn original_imported_abi_version_ranked_and_empty_query_order() {
        assert_eq!(rustcast_abi_version(), 1);
        let catalog = CString::new(
            r#"[{"id":"first","title":"Same","kind":"url","target":"https://example.com"},{"id":"second","title":"Same","kind":"url","target":"https://example.com"}]"#,
        )
        .unwrap();
        let index = IndexGuard(unsafe { rustcast_index_new(catalog.as_ptr()) });
        assert!(!index.0.is_null());
        drop(catalog);

        let recency = [0.0, 5.0];
        let mut output = [usize::MAX; 2];
        let query = b"same";
        let count = unsafe {
            rustcast_index_search_ranked_into(
                index.0,
                query.as_ptr(),
                query.len(),
                recency.as_ptr(),
                recency.len(),
                output.as_mut_ptr(),
                output.len(),
            )
        };
        assert_eq!(count, 2);
        assert_eq!(output, [1, 0]);

        output = [usize::MAX; 2];
        let count = unsafe {
            rustcast_index_search_ranked_into(
                index.0,
                b"".as_ptr(),
                0,
                recency.as_ptr(),
                recency.len(),
                output.as_mut_ptr(),
                output.len(),
            )
        };
        assert_eq!(count, 2);
        assert_eq!(output, [0, 1]);
    }

    #[test]
    fn snippet_output_modes_roundtrip_validate_and_reject_unknown() {
        let source = "Hi 👋\n\nSee [Guide & tips](https://example.com/a_(b)?x=1&y=2)\nGoodbye\n";
        for mode in ["plain", "teams", "html"] {
            let command: Command = serde_json::from_value(serde_json::json!({
                "id": "fixture.rich",
                "title": "Rich",
                "kind": "snippet",
                "target": source,
                "snippetOutput": mode
            }))
            .unwrap();
            let encoded = serde_json::to_vec(&command).unwrap();
            let copy: Command = serde_json::from_slice(&encoded).unwrap();
            assert_eq!(
                serde_json::to_value(&copy).unwrap(),
                serde_json::to_value(&command).unwrap()
            );

            let mut config = default_config();
            config["commands"] = serde_json::json!([command]);
            assert!(validate(&config).is_ok(), "{mode}");
        }

        let mut invalid = default_config();
        invalid["commands"] = serde_json::json!([{
            "id": "bad",
            "title": "Bad",
            "kind": "snippet",
            "target": "x",
            "snippetOutput": "other"
        }]);
        assert!(validate(&invalid).is_err());
    }

    #[test]
    fn original_duplicate_snippet_keyword_is_rejected() {
        let first = json!({
            "id": "w",
            "title": "Word",
            "kind": "snippet",
            "target": "expanded",
            "keyword": "abc"
        });
        let mut config = default_config();
        config["commands"].as_array_mut().unwrap().push(first);
        assert!(validate(&config).is_ok());

        config["commands"].as_array_mut().unwrap().push(json!({
            "id": "w2",
            "title": "Word 2",
            "kind": "snippet",
            "target": "other",
            "keyword": "abc"
        }));
        let error = validate(&config)
            .err()
            .expect("duplicate snippet keywords must be rejected");
        assert_eq!(error, "Duplicate or non-snippet keyword.");
    }

    #[test]
    fn typed_search_respects_capacity_and_rejects_invalid_input() {
        let commands = validate(&default_config()).unwrap().commands;
        let index = SearchIndex::new(&commands);
        let mut output = [usize::MAX; 3];
        unsafe {
            assert_eq!(
                rustcast_index_search_into(&index, b"".as_ptr(), 0, output.as_mut_ptr(), 2),
                2
            );
            assert_eq!(output, [0, 1, usize::MAX]);
            assert_eq!(
                rustcast_index_search_into(&index, b"".as_ptr(), 0, std::ptr::null_mut(), 0),
                0
            );
            assert_eq!(
                rustcast_index_search_into(&index, b"".as_ptr(), 0, std::ptr::null_mut(), 1),
                -1
            );
            assert_eq!(
                rustcast_index_search_into(&index, [255].as_ptr(), 1, output.as_mut_ptr(), 3),
                -1
            );
            assert_eq!(
                rustcast_index_search_into(
                    std::ptr::null(),
                    b"x".as_ptr(),
                    1,
                    output.as_mut_ptr(),
                    3
                ),
                -1
            );
            let weights = vec![f64::NAN; commands.len()];
            assert_eq!(
                rustcast_index_search_ranked_into(
                    &index,
                    b"x".as_ptr(),
                    1,
                    weights.as_ptr(),
                    weights.len(),
                    output.as_mut_ptr(),
                    3
                ),
                -1
            );
            assert_eq!(
                rustcast_index_search_ranked_into(
                    &index,
                    b"x".as_ptr(),
                    1,
                    weights.as_ptr(),
                    1,
                    output.as_mut_ptr(),
                    3
                ),
                -1
            );
        }
    }
    #[test]
    fn original_script_and_settings_validation_rejections() {
        let mut relative_script = default_config();
        relative_script["commands"].as_array_mut().unwrap().push(json!({
            "id": "s",
            "title": "Script",
            "kind": "script",
            "target": "relative"
        }));
        assert!(validate(&relative_script).is_err());

        let mut oversized_clipboard = default_config();
        oversized_clipboard["clipboard"]["limit"] = json!(1001);
        assert!(validate(&oversized_clipboard).is_err());
        assert!(shortcut("ctrl+ctrl+v").is_err());
        assert!(shortcut("space").is_err());
    }

    #[test]
    fn default_is_valid() {
        validate(&default_config()).unwrap();
    }
    #[test]
    fn original_empty_catalog_search_returns_no_commands() {
        let index = SearchIndex::new(&[]);
        assert!(index.search("").is_empty());
    }
    #[test]
    fn original_unicode_scalar_and_expanded_lowercase_search() {
        let commands: Vec<Command> = serde_json::from_value(json!([
            {"id":"scalar","title":"A Ω B","kind":"url","target":"https://example.com"},
            {"id":"expanded","title":"I x \u{0307}","kind":"url","target":"https://example.com"}
        ]))
        .unwrap();
        let index = SearchIndex::new(&commands);
        for (query, expected) in [("Ω", "scalar"), ("İ", "expanded")] {
            let ids = index
                .search(query)
                .iter()
                .map(|position| commands[*position].id.as_str())
                .collect::<Vec<_>>();
            assert_eq!(ids, [expected], "query {query}");
        }
    }
    #[test]
    fn original_large_equal_title_ties_preserve_order_and_alias_precedence() {
        let commands: Vec<Command> = (0..128)
            .map(|i| {
                let mut command = json!({
                    "id": format!("tie.{i}"),
                    "title": "Same Title",
                    "kind": "url",
                    "target": "https://example.com"
                });
                if i == 127 {
                    command["aliases"] = json!(["same"]);
                }
                serde_json::from_value(command).unwrap()
            })
            .collect();
        let index = SearchIndex::new(&commands);
        let expected = (0..128).map(|i| format!("tie.{i}")).collect::<Vec<_>>();
        let ids = |positions: Vec<usize>| {
            positions
                .into_iter()
                .map(|position| commands[position].id.as_str())
                .collect::<Vec<_>>()
        };

        assert_eq!(
            ids(index.search("title")),
            expected.iter().map(String::as_str).collect::<Vec<_>>()
        );
        let mut recency = vec![0.0; 128];
        recency[64] = 100.0;
        let same = ids(index.search_with_recency("same", &recency).unwrap());
        assert_eq!(&same[..3], ["tie.127", "tie.64", "tie.0"]);
        assert_eq!(
            ids(index.search("s t")),
            expected.iter().map(String::as_str).collect::<Vec<_>>()
        );
    }

    #[test]
    fn original_embedded_nul_alias_search_returns_matching_command() {
        let commands: Vec<Command> = serde_json::from_value(json!([
            {"id":"z","title":"Beta","kind":"url","target":"https://example.com/one","aliases":["go"]},
            {"id":"a","title":"Go Alpha","kind":"url","target":"https://example.com/two"},
            {"id":"b","title":"Ångström Notes","kind":"url","target":"https://example.com/three","aliases":["nul\0alias"]},
            {"id":"c","title":"Beta","kind":"url","target":"https://example.com/four"}
        ]))
        .unwrap();
        let index = SearchIndex::new(&commands);
        let ids = index
            .search("nul\0alias")
            .iter()
            .map(|position| commands[*position].id.as_str())
            .collect::<Vec<_>>();
        assert_eq!(ids, ["b"]);
    }

    #[test]
    fn original_nonempty_catalog_missing_query_returns_no_commands() {
        let commands: Vec<Command> = serde_json::from_value(json!([
            {"id":"z","title":"Beta","kind":"url","target":"https://example.com/one","aliases":["go"]},
            {"id":"a","title":"Go Alpha","kind":"url","target":"https://example.com/two"},
            {"id":"b","title":"Ångström Notes","kind":"url","target":"https://example.com/three","aliases":["nul\0alias"]},
            {"id":"c","title":"Beta","kind":"url","target":"https://example.com/four"}
        ]))
        .unwrap();
        let index = SearchIndex::new(&commands);
        assert!(index.search("does not exist").is_empty());
    }

    #[test]
    fn original_blank_queries_preserve_nonempty_catalog_order() {
        let commands: Vec<Command> = serde_json::from_value(json!([
            {"id":"z","title":"Beta","kind":"url","target":"https://example.com/one","aliases":["go"]},
            {"id":"a","title":"Go Alpha","kind":"url","target":"https://example.com/two"},
            {"id":"b","title":"Ångström Notes","kind":"url","target":"https://example.com/three","aliases":["nul\0alias"]},
            {"id":"c","title":"Beta","kind":"url","target":"https://example.com/four"}
        ]))
        .unwrap();
        let index = SearchIndex::new(&commands);
        let expected = ["z", "a", "b", "c"];
        let ids = |positions: Vec<usize>| {
            positions
                .into_iter()
                .map(|position| commands[position].id.as_str())
                .collect::<Vec<_>>()
        };

        assert_eq!(ids(index.search("   ")), expected);
        assert_eq!(
            ids(index.search_with_recency("", &[999.0, 0.0, 0.0, 0.0]).unwrap()),
            expected
        );
    }

    #[test]
    fn original_quality_ranking_alias_recency_and_multi_term_search() {
        let commands: Vec<Command> = serde_json::from_value(json!([
            {"id":"exact","title":"Zulu","kind":"url","target":"https://example.com","aliases":["mt"]},
            {"id":"teams","title":"Microsoft Teams","kind":"url","target":"https://example.com"},
            {"id":"recent","title":"Many Tabs","kind":"url","target":"https://example.com"},
            {"id":"history","title":"Microsoft Teams History","kind":"url","target":"https://example.com"}
        ]))
        .unwrap();
        let index = SearchIndex::new(&commands);
        let ids = |positions: Vec<usize>| {
            positions
                .into_iter()
                .map(|position| commands[position].id.as_str())
                .collect::<Vec<_>>()
        };

        assert_eq!(
            &ids(index.search_with_recency("mt", &[0.0, 0.0, 100.0, 0.0]).unwrap())[..3],
            ["exact", "recent", "teams"]
        );
        assert_eq!(ids(index.search("mic tea"))[0], "teams");
        assert_eq!(ids(index.search("mft"))[0], "teams");
        assert_eq!(
            ids(index.search_with_recency("Microsoft Teams", &[0.0, 0.0, 0.0, 100.0]).unwrap())[0],
            "teams"
        );
    }

    #[test]
    fn duplicate_aliases_and_modifiers_fail() {
        let mut v = default_config();
        v["commands"][1]["aliases"] = json!(["FiNdEr"]);
        assert!(validate(&v).is_err());
        assert!(shortcut("ctrl+ctrl+v").is_err());
        assert_eq!(
            shortcut("ctrl+alt+space").unwrap(),
            shortcut("alt+ctrl+space").unwrap()
        );
        let mut reserved_hotkey = default_config();
        reserved_hotkey["commands"][0]["hotkey"] = json!("alt+ctrl+space");
        assert!(validate(&reserved_hotkey).is_err());
        let mut unsupported_version = default_config();
        unsupported_version["version"] = json!(2);
        assert!(validate(&unsupported_version).is_err());
    }
    #[test]
    fn command_sides_are_independent() {
        let mut legacy = default_config();
        assert_eq!(
            validate(&legacy).unwrap().separate_command_keys,
            Some(false)
        );
        legacy
            .as_object_mut()
            .unwrap()
            .remove("separateCommandKeys");
        assert_eq!(validate(&legacy).unwrap().separate_command_keys, None);
        legacy["separateCommandKeys"] = json!(true);
        assert_eq!(validate(&legacy).unwrap().separate_command_keys, Some(true));
        legacy["separateCommandKeys"] = json!(false);
        assert_eq!(validate(&legacy).unwrap().separate_command_keys, Some(false));
        legacy["separateCommandKeys"] = json!("true");
        assert!(validate(&legacy).is_err());
        assert_eq!(shortcut("cmd+a").unwrap(), "cmd+a");
        assert_eq!(shortcut("RIGHT-CMD+a").unwrap(), "right-cmd+a");
        assert_eq!(shortcut("shift+left-cmd+a").unwrap(), "left-cmd+shift+a");
        for invalid in [
            "cmd+right-cmd+a",
            "left-cmd+right-cmd+a",
            "right-cmd+right-cmd+a",
        ] {
            assert!(shortcut(invalid).is_err());
        }
        let mut v = default_config();
        for (i, binding) in ["cmd+a", "right-cmd+a", "left-cmd+a"].iter().enumerate() {
            v["commands"][i]["hotkey"] = json!(binding);
        }
        validate(&v).unwrap();
        v["commands"][1]["hotkey"] = json!("left-cmd+a");
        assert!(validate(&v).is_err());
    }
    #[test]
    fn rejects_unknown_config_and_relative_scripts() {
        let mut v = default_config();
        v["typo"] = json!(true);
        assert!(validate(&v).is_err());
        v.as_object_mut().unwrap().remove("typo");
        v["commands"][0]["kind"] = json!("script");
        v["commands"][0]["target"] = json!("relative");
        assert!(validate(&v).is_err())
    }
    #[test]
    fn exact_alias_ranks_first() {
        let c = validate(&default_config()).unwrap();
        assert_eq!(search(&c.commands, "finder")[0].id, "app.finder");
    }
    #[test]
    fn search_intent_fixtures() {
        let fixture: Value =
            serde_json::from_str(include_str!("../../Tests/fixtures/search-quality.json")).unwrap();
        let commands: Vec<Command> = serde_json::from_value(fixture["commands"].clone()).unwrap();
        let index = SearchIndex::new(&commands);
        for case in fixture["cases"].as_array().unwrap() {
            let query = case["query"].as_str().unwrap();
            let got = index.search(query);
            assert_eq!(
                got.first().map(|i| commands[*i].id.as_str()),
                case["intended"].as_str(),
                "query {query}"
            );
            assert_eq!(index.search(query), got, "determinism for {query}");
        }
    }
    #[test]
    fn original_256_command_ten_query_index_dispatch_and_abi_parity() {
        let titles = [
            "Microsoft Teams",
            "Open Documentation",
            "Left Half",
            "Project Dashboard",
        ];
        let commands: Vec<Command> = (0..256)
            .map(|i| {
                serde_json::from_value(json!({
                    "id": format!("item.{i}"),
                    "title": format!("{} {i}", titles[i % titles.len()]),
                    "kind": "url",
                    "target": format!("https://example.com/{i}"),
                    "aliases": [format!("a{i}")]
                }))
                .unwrap()
            })
            .collect();
        let index = SearchIndex::new(&commands);
        let queries = [
            "",
            " ",
            "MICROSOFT",
            "a42",
            "item.11",
            "project dashboard",
            "example.com/42",
            "missing",
            "Ångström",
            "a\0b",
        ];

        for query in queries {
            let indices = index.search(query);
            let indexed: Vec<Command> = indices
                .iter()
                .map(|&index| commands[index].clone())
                .collect();
            let indexed = serde_json::to_value(indexed).unwrap();
            let dispatched = dispatch(
                "search",
                json!({"commands": commands, "query": query}),
            )
            .unwrap();
            assert_eq!(dispatched, indexed, "index/dispatch parity for {query:?}");

            let mut abi = vec![usize::MAX; commands.len()];
            let count = unsafe {
                rustcast_index_search_ranked_into(
                    &index,
                    query.as_ptr(),
                    query.len(),
                    std::ptr::null(),
                    0,
                    abi.as_mut_ptr(),
                    abi.len(),
                )
            };
            assert_eq!(count, indices.len() as isize, "ABI result count for {query:?}");
            assert_eq!(&abi[..count as usize], indices, "ABI order for {query:?}");
        }
    }
    #[test]
    fn direct_title_typo_outranks_weak_subsequence_with_higher_recency() {
        let commands: Vec<Command> = [
            ("typo.finder", "Finder"),
            ("typo.distractor", "Find Reminders"),
        ]
        .into_iter()
        .map(|(id, title)| {
            serde_json::from_value(json!({
                "id": id,
                "title": title,
                "kind": "url",
                "target": "https://example.com"
            }))
            .unwrap()
        })
        .collect();
        let index = SearchIndex::new(&commands);
        let results = index.search_with_recency("fnider", &[0.0, 10_000.0]).unwrap();
        let ids: Vec<&str> = results
            .iter()
            .map(|&entry| commands[entry].id.as_str())
            .collect();
        assert_eq!(ids, ["typo.finder", "typo.distractor"]);
    }
    #[test]
    fn original_bounded_typo_fallback_search_cases() {
        fn command(id: &str, title: &str, kind: &str, target: &str, aliases: &[&str]) -> Command {
            serde_json::from_value(json!({
                "id": id,
                "title": title,
                "kind": kind,
                "target": target,
                "aliases": aliases,
            }))
            .unwrap()
        }
        fn ids<'a>(commands: &'a [Command], results: Vec<usize>) -> Vec<&'a str> {
            results
                .iter()
                .map(|index| commands[*index].id.as_str())
                .collect()
        }

        let commands = vec![
            command("finder", "Finder", "app", "com.apple.finder", &[]),
            command("calendar", "Calendar", "app", "com.apple.iCal", &[]),
            command("alias", "Zulu", "url", "https://example.com", &["fnider"]),
        ];
        let index = SearchIndex::new(&commands);
        assert_eq!(
            ids(
                &commands,
                index.search_with_recency("fnider", &[999.0, 0.0, 0.0]).unwrap()
            ),
            ["alias"]
        );
        assert_eq!(ids(&commands, index.search("calender")), ["calendar"]);

        let ties = vec![
            command("a", "Finder", "url", "https://example.com/a", &[]),
            command("b", "Finder", "url", "https://example.com/b", &[]),
        ];
        let index = SearchIndex::new(&ties);
        assert_eq!(ids(&ties, index.search("fnider")), ["a", "b"]);
        assert_eq!(
            ids(&ties, index.search_with_recency("fnider", &[0.0, 100.0]).unwrap()),
            ["b", "a"]
        );

        let ambiguous = vec![
            command("mail", "Mail", "url", "https://example.com", &[]),
            command("main", "Main", "url", "https://example.com", &[]),
        ];
        let index = SearchIndex::new(&ambiguous);
        assert_eq!(ids(&ambiguous, index.search("maik")), ["mail", "main"]);
        assert_eq!(
            ids(
                &ambiguous,
                index.search_with_recency("maik", &[0.0, 100.0]).unwrap()
            ),
            ["main", "mail"]
        );

        let weak_sequence = vec![
            commands[0].clone(),
            command("reminders", "Reminders", "url", "https://example.com", &[]),
            command(
                "distractor",
                "Find Reminders",
                "url",
                "https://example.com",
                &[],
            ),
        ];
        let index = SearchIndex::new(&weak_sequence);
        assert_eq!(ids(&weak_sequence, index.search("fnider")), ["finder", "distractor"]);
        assert_eq!(
            ids(
                &weak_sequence,
                index
                    .search_with_recency("fnider", &[0.0, 0.0, 999.0])
                    .unwrap()
            ),
            ["finder", "distractor"]
        );
        assert_eq!(
            ids(&weak_sequence, index.search("remidners")),
            ["reminders", "distractor"]
        );
        assert_eq!(
            ids(
                &weak_sequence,
                index
                    .search_with_recency("remidners", &[0.0, 0.0, 999.0])
                    .unwrap()
            ),
            ["reminders", "distractor"]
        );

        let decorated = vec![
            command("plain", "Notes", "url", "https://example.com", &[]),
            command("decorated", "Notes!", "url", "https://example.com", &[]),
        ];
        let index = SearchIndex::new(&decorated);
        assert_eq!(
            ids(&decorated, index.search_with_recency("noets", &[0.0, 999.0]).unwrap()),
            ["plain", "decorated"]
        );
        assert!(index.search("noets!").is_empty());

        let weak = [
            commands.clone(),
            vec![command(
                "weak",
                "Other",
                "url",
                "https://example.com/calender",
                &[],
            )],
        ]
        .concat();
        let index = SearchIndex::new(&weak);
        assert_eq!(ids(&weak, index.search("calender")), ["weak"]);

        let short = vec![command("short", "Cat", "url", "https://example.com", &[])];
        assert!(SearchIndex::new(&short).search("cta").is_empty());

        let unicode = vec![command("word", "Åbcd", "url", "https://example.com", &[])];
        let index = SearchIndex::new(&unicode);
        assert_eq!(ids(&unicode, index.search("Åbdc")), ["word"]);
        assert!(index.search("Åb\u{0301}dc").is_empty());

        for (length, expected) in [(64, true), (65, false)] {
            let title = format!("{}bc", "a".repeat(length - 2));
            let typo = format!("{}cb", "a".repeat(length - 2));
            let long = vec![command("long", &title, "url", "https://example.com", &[])];
            let results = SearchIndex::new(&long).search(&typo);
            assert_eq!(ids(&long, results), if expected { vec!["long"] } else { vec![] });
        }
    }
    #[test]
    fn single_typo_matches_independent_edit_neighborhood() {
        fn strings(n: usize) -> Vec<String> {
            let mut all = vec![String::new()];
            let mut row = vec![String::new()];
            for _ in 0..n {
                row = row
                    .iter()
                    .flat_map(|s| ['a', 'b', 'Ω'].map(|c| format!("{s}{c}")))
                    .collect();
                all.extend(row.clone());
            }
            all
        }
        let samples = strings(4);
        for a in &samples {
            let chars = a.chars().collect::<Vec<_>>();
            let mut neighbors = std::collections::HashSet::from([a.clone()]);
            for i in 0..=chars.len() {
                for c in ['a', 'b', 'Ω'] {
                    let mut t = chars.clone();
                    t.insert(i, c);
                    neighbors.insert(t.iter().collect::<String>());
                }
                if i == chars.len() {
                    continue;
                }
                let mut t = chars.clone();
                t.remove(i);
                neighbors.insert(t.iter().collect::<String>());
                for c in ['a', 'b', 'Ω'] {
                    let mut t = chars.clone();
                    t[i] = c;
                    neighbors.insert(t.iter().collect::<String>());
                }
                if i + 1 < chars.len() {
                    let mut t = chars.clone();
                    t.swap(i, i + 1);
                    neighbors.insert(t.iter().collect::<String>());
                }
            }
            for b in &samples {
                assert_eq!(one_typo(a, b), neighbors.contains(b), "{a:?} vs {b:?}");
            }
        }
    }
    #[test]
    fn invalid_ipc_json_is_error_not_panic() {
        unsafe {
            let op = CString::new("validate").unwrap();
            let input = CString::new("{").unwrap();
            let r = rustcast_call(op.as_ptr(), input.as_ptr());
            let s = CStr::from_ptr(r).to_str().unwrap();
            assert!(s.contains("\"ok\":false"));
            rustcast_free(r)
        }
    }
    #[test]
    fn negative_display_coordinates() {
        let frame = Frame {
            x: -3440.,
            y: -200.,
            width: 3440.,
            height: 1400.,
        };
        let f = layout(frame, "last-two-thirds").unwrap();
        assert!((f.x - (-3440. + 3440. / 3.)).abs() < 0.01);
        assert!((f.width - (3440. * 2. / 3.)).abs() < 0.01);
        assert!(layout(frame, "other").is_err());
    }
    #[test]
    fn history_bounds_expiry_and_secrets() {
        let s = ClipboardSettings {
            enabled: true,
            limit: 2,
            max_text_bytes: 20,
            retention_days: 1,
            excluded_apps: vec!["secret".into()],
            max_asset_bytes: None,
            max_total_asset_bytes: None,
        };
        let now = "2026-10-24T10:00:00Z";
        let clip = |id: &str, text: &str, date: &str, source: &str| Clip {
            id: id.into(),
            text: text.into(),
            date: date.into(),
            source: source.into(),
            content_type: None,
            data_file: None,
            file_urls: None,
        };
        let b = clip("b", "b", now, "app");
        let a = clip("a", "a", now, "app");
        let expired = clip("expired", "expired", "2026-10-23T09:00:00Z", "app");
        let ordered = history(&[b, a.clone(), expired], Some(a.clone()), &s, now).unwrap();
        assert_eq!(
            ordered.iter().map(|item| item.text.as_str()).collect::<Vec<_>>(),
            ["a", "b"]
        );
        assert_eq!(
            serde_json::to_value(&ordered).unwrap(),
            serde_json::to_value([a.clone(), clip("b", "b", now, "app")]).unwrap()
        );
        assert!(history(&[], Some(clip("blocked", "password", now, "secret")), &s, now)
            .unwrap()
            .is_empty());
        assert!(history(
            &[clip("expired-secret", "hello", now, "secret")],
            None,
            &s,
            "2026-10-26T11:00:00Z"
        )
        .unwrap()
        .is_empty());

        for rejected in [
            clip("password", "password", now, "secret"),
            clip("oversized", &"x".repeat(21), now, "app"),
            clip("blank", "  ", now, "app"),
        ] {
            let unchanged = history(&ordered, Some(rejected), &s, now).unwrap();
            assert_eq!(
                serde_json::to_value(&unchanged).unwrap(),
                serde_json::to_value(&ordered).unwrap()
            );
        }
    }
}
