//! Shared Rustcast domain logic. The Mac host is a native platform adapter.
use chrono::DateTime;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::HashSet,
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
    #[serde(default, skip_serializing_if = "Option::is_none", rename = "applicationID")]
    pub application_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
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
pub struct Config {
    pub version: u32,
    pub launcher_hotkey: String,
    pub clipboard_hotkey: String,
    pub clipboard: ClipboardSettings,
    #[serde(default)]
    pub snippet_expansion_enabled: bool,
    #[serde(default)]
    pub hyper_key_enabled: bool,
    pub commands: Vec<Command>,
}
pub fn default_config() -> Value {
    serde_json::from_str(include_str!("../../config/default.json")).expect("shipped default config")
}
pub fn shortcut(s: &str) -> Result<String, String> {
    let lower = s.to_lowercase();
    let mut parts = lower.split('+').collect::<Vec<_>>();
    let key = parts.pop().unwrap_or("");
    let known_key = key.len() == 1
        && (key.as_bytes()[0].is_ascii_alphanumeric() || "[],./-=\\';`".contains(key))
        || ["space", "tab", "return", "left", "right", "up", "down", "left-control", "left-option", "delete", "backspace", "escape", "f1", "f2", "f3", "f4", "f5", "f6", "f7", "f8", "f9", "f10", "f11", "f12"].contains(&key);
    let mut mods = HashSet::new();
    if !known_key
        || parts.is_empty()
        || parts
            .iter()
            .any(|m| !["ctrl", "alt", "shift", "cmd"].contains(m) || !mods.insert(*m))
    {
        return Err(format!("Invalid hotkey: {s}. Use e.g. ctrl+alt+space."));
    }
    parts.sort();
    Ok(format!("{}+{key}", parts.join("+")))
}
pub fn validate(value: &Value) -> Result<Config, String> {
    let c: Config = serde_json::from_value(value.clone()).map_err(|e| e.to_string())?;
    if c.version != 1 {
        return Err("Unsupported config version; expected 1.".into());
    }
    if !(1..=1000).contains(&c.clipboard.limit)
        || !(1..=1_000_000).contains(&c.clipboard.max_text_bytes)
        || !(1..=365).contains(&c.clipboard.retention_days)
    {
        return Err(
            "Clipboard bounds: limit 1–1000, maxTextBytes 1–1000000, retentionDays 1–365.".into(),
        );
    }
    if c.clipboard.max_asset_bytes.is_some_and(|n|n==0 || n>100_000_000) || c.clipboard.max_total_asset_bytes.is_some_and(|n|n==0 || n>1_000_000_000) {
        return Err("Clipboard image limits must be positive, maxAssetBytes <=100MB and maxTotalAssetBytes <=1GB.".into());
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
                    "create-quicklink", "settings", "analytics", "search-files", "search-menu-items", "switch-windows", "kill-process", "system-monitor", "lock-screen", "utility-set-text", "utility-paste-text", "utility-text-history", "utility-password", "utility-username", "utility-password-history", "utility-username-history",
                ]
                .contains(&cmd.target.as_str())
                {
                    return Err(format!("Unknown built-in: {}", cmd.target));
                }
            }
            _ => return Err(format!("Unknown command kind: {}", cmd.kind)),
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
}
impl SearchIndex {
    pub fn new(commands: &[Command]) -> Self {
        Self {
            entries: commands
                .iter()
                .map(|c| SearchEntry {
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
        }
    }
    pub fn search(&self, query: &str) -> Vec<usize> {
        let q = query.trim().to_lowercase();
        if q.is_empty() {
            return (0..self.entries.len()).collect();
        }
        let terms = q.split_whitespace().collect::<Vec<_>>();
        let mut found = self
            .entries
            .iter()
            .enumerate()
            .filter_map(|(i, e)| {
                let score = if e.id == q || e.aliases.contains(&q) {
                    0
                } else if e.title.starts_with(&q) {
                    1
                } else if terms.iter().all(|w| e.haystack.contains(w)) {
                    2
                } else {
                    return None;
                };
                Some((score, i))
            })
            .collect::<Vec<_>>();
        found.sort_by(|a, b| {
            a.0.cmp(&b.0)
                .then(self.entries[a.1].title.cmp(&self.entries[b.1].title))
        });
        found.into_iter().map(|(_, i)| i).collect()
    }
}
pub fn search(commands: &[Command], query: &str) -> Vec<Command> {
    SearchIndex::new(commands)
        .search(query)
        .into_iter()
        .map(|i| commands[i].clone())
        .collect()
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
    #[serde(default, skip_serializing_if = "Option::is_none", rename = "contentType")]
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
            kept.retain(|x| x.id != c.id && !(x.text == c.text && x.content_type == c.content_type && x.data_file == c.data_file && x.file_urls == c.file_urls));
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
/// Both pointers must be non-null, NUL-terminated strings valid for this call.
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
    #[test]
    fn default_is_valid() {
        validate(&default_config()).unwrap();
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
        let f = layout(
            Frame {
                x: -3440.,
                y: -200.,
                width: 3440.,
                height: 1400.,
            },
            "last-two-thirds",
        )
        .unwrap();
        assert!((f.x - (-3440. + 3440. / 3.)).abs() < 0.01);
    }
    #[test]
    fn history_bounds_expiry_and_secrets() {
        let s = ClipboardSettings {
            enabled: true,
            limit: 2,
            max_text_bytes: 20,
            retention_days: 1,
            excluded_apps: vec!["secret".into()], max_asset_bytes:None,max_total_asset_bytes:None,
        };
        let c = Clip {
            id: "1".into(),
            text: "hello".into(),
            date: "2026-10-24T10:00:00Z".into(),
            source: "secret".into(),content_type:None,data_file:None,file_urls:None,
        };
        assert!(history(&[], Some(c.clone()), &s, "2026-10-24T11:00:00Z")
            .unwrap()
            .is_empty());
        assert!(history(&[c], None, &s, "2026-10-26T11:00:00Z")
            .unwrap()
            .is_empty());
    }
}
