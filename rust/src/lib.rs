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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub separate_command_keys: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub theme: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub menu_bar_visible: Option<bool>,
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
            .any(|m| !["ctrl", "alt", "shift", "cmd", "left-cmd", "right-cmd"].contains(m) || !mods.insert(*m))
    {
        return Err(format!("Invalid hotkey: {s}. Use e.g. ctrl+alt+space."));
    }
    if ["cmd", "left-cmd", "right-cmd"].iter().filter(|m| mods.contains(**m)).count() > 1 {
        return Err("Choose cmd, left-cmd or right-cmd, not a combination.".into());
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
    if c.theme.as_ref().is_some_and(|theme| !["system","light","dark","graphite"].contains(&theme.as_str())) {return Err("Theme must be system, light, dark or graphite.".into());}
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
                    "create-quicklink", "settings", "open-config", "reload-config", "analytics", "search-files", "search-menu-items", "switch-windows", "apple-shortcuts", "kill-process", "system-monitor", "lock-screen", "utility-set-text", "utility-paste-text", "utility-text-history", "utility-password", "utility-username", "utility-password-history", "utility-username-history",
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
    words: Vec<String>,
    initials: String,
    title_order:usize,
}
impl SearchIndex {
    pub fn new(commands: &[Command]) -> Self {
        // Resolve stable title/catalog order once at the immutable catalog boundary.
        let mut result=Self {
            entries: commands
                .iter()
                .map(|c| SearchEntry {title_order:0,
                    words: c.title.to_lowercase().split(|c:char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).map(str::to_owned).collect(),
                    initials: c.title.to_lowercase().split(|c:char| !c.is_alphanumeric()).filter_map(|w| w.chars().next()).collect(),
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
        let mut order=(0..result.entries.len()).collect::<Vec<_>>();
        order.sort_by(|a,b|result.entries[*a].title.cmp(&result.entries[*b].title));
        for (rank,index) in order.into_iter().enumerate() {result.entries[index].title_order=rank;}
        result
    }
    pub fn search(&self, query: &str) -> Vec<usize> {
        self.search_ranked(query,&[])
    }
    fn search_ranked(&self, query:&str, recency:&[f64]) -> Vec<usize> {
        let q = query.trim().to_lowercase();
        if q.is_empty() {
            return (0..self.entries.len()).collect();
        }
        let terms = q.split_whitespace().collect::<Vec<_>>();
        let fuzzy=q.chars().nth(1).is_some();
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
                } else if terms.iter().all(|w| e.words.iter().any(|word|word.starts_with(w))) || (fuzzy && e.initials.starts_with(&q)) {
                    3
                } else if terms.iter().all(|w| e.haystack.contains(w)) {
                    4
                } else if fuzzy && {
                    let mut title=e.title.chars();
                    q.chars().filter(|c| !c.is_whitespace()).all(|needle| title.any(|c|c==needle))
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
        if found.iter().all(|(score,_)|*score==5) && (4..=64).contains(&q.chars().take(65).count()) && q.chars().all(char::is_alphanumeric) {
            let had_weak=!found.is_empty();
            found.clear();
            found.extend(self.entries.iter().enumerate().filter_map(|(i,e)| {
                // This length check identifies a complete single-word title
                // without re-scanning long strings or storing more index data.
                let whole=e.words.len()==1 && e.words[0].len()==e.title.len();
                let score=if whole && one_typo(&q,&e.title) {5}
                    else if !whole && e.words.iter().any(|word|one_typo(&q,word)) {6}
                    else if had_weak && {
                        let mut title=e.title.chars();
                        q.chars().all(|needle|title.any(|c|c==needle))
                    } {7} else {return None;};
                Some((score,i))
            }));
        }
        found.sort_by(|a, b| {
            a.0.cmp(&b.0)
                .then_with(||recency.get(b.1).unwrap_or(&0.0).total_cmp(recency.get(a.1).unwrap_or(&0.0)))
                .then_with(||self.entries[a.1].title_order.cmp(&self.entries[b.1].title_order))
        });
        found.into_iter().map(|(_, i)| i).collect()
    }
}
// One substitution, insertion, deletion or adjacent transposition, in Unicode
// scalars. Iterator comparisons avoid a dynamic-programming matrix/allocation.
fn one_typo(a:&str,b:&str)->bool {
    let mut a=a.chars();let mut b=b.chars();
    loop {
        match (a.next(),b.next()) {
            (None,None)=>return true,
            (None,Some(_))=>return b.next().is_none(),
            (Some(_),None)=>return a.next().is_none(),
            (Some(x),Some(y)) if x==y=>{},
            (Some(x),Some(y))=>{
                if a.clone().eq(b.clone()) || a.clone().eq(std::iter::once(y).chain(b.clone()))
                    || std::iter::once(x).chain(a.clone()).eq(b.clone()) {return true;}
                return a.next()==Some(y) && b.next()==Some(x) && a.eq(b);
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
pub extern "C" fn rustcast_abi_version() -> u32 { 1 }

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
    index: *const SearchIndex, query: *const u8, length: usize,
    output: *mut usize, capacity: usize,
) -> isize {
    rustcast_index_search_ranked_into(index,query,length,std::ptr::null(),0,output,capacity)
}
/// # Safety
/// Same requirements as rustcast_index_search_into; recency is either omitted
/// (count 0) or a readable finite nonnegative f64 weight per catalog entry.
#[no_mangle]
pub unsafe extern "C" fn rustcast_index_search_ranked_into(
    index:*const SearchIndex,query:*const u8,length:usize,
    recency:*const f64,recency_count:usize,output:*mut usize,capacity:usize,
) -> isize {
    std::panic::catch_unwind(|| {
        if index.is_null() || query.is_null() || (capacity > 0 && output.is_null()) {
            return -1;
        }
        let Ok(q) = std::str::from_utf8(std::slice::from_raw_parts(query, length)) else { return -1; };
        let weights=if recency_count==0 {&[][..]} else {
            if recency.is_null() || recency_count!=(*index).entries.len() {return -1;}
            let weights=std::slice::from_raw_parts(recency,recency_count);
            if weights.iter().any(|v| !v.is_finite() || *v<0.0) {return -1;}
            weights
        };
        let found = (*index).search_ranked(q,weights);
        let count = found.len().min(capacity);
        if count > 0 { std::ptr::copy_nonoverlapping(found.as_ptr(), output, count); }
        count as isize
    }).unwrap_or(-1)
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
    #[test]
    fn typed_search_respects_capacity_and_rejects_invalid_input() {
        let commands=validate(&default_config()).unwrap().commands;
        let index=SearchIndex::new(&commands);
        let mut output=[usize::MAX;3];
        unsafe {
            assert_eq!(rustcast_index_search_into(&index,b"".as_ptr(),0,output.as_mut_ptr(),2),2);
            assert_eq!(output,[0,1,usize::MAX]);
            assert_eq!(rustcast_index_search_into(&index,b"".as_ptr(),0,std::ptr::null_mut(),0),0);
            assert_eq!(rustcast_index_search_into(&index,b"".as_ptr(),0,std::ptr::null_mut(),1),-1);
            assert_eq!(rustcast_index_search_into(&index,[255].as_ptr(),1,output.as_mut_ptr(),3),-1);
            assert_eq!(rustcast_index_search_into(std::ptr::null(),b"x".as_ptr(),1,output.as_mut_ptr(),3),-1);
            let weights=vec![f64::NAN;commands.len()];
            assert_eq!(rustcast_index_search_ranked_into(&index,b"x".as_ptr(),1,weights.as_ptr(),weights.len(),output.as_mut_ptr(),3),-1);
            assert_eq!(rustcast_index_search_ranked_into(&index,b"x".as_ptr(),1,weights.as_ptr(),1,output.as_mut_ptr(),3),-1);
        }
    }
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
    fn command_sides_are_independent() {
        let mut legacy=default_config();
        assert_eq!(validate(&legacy).unwrap().separate_command_keys,Some(false));
        legacy.as_object_mut().unwrap().remove("separateCommandKeys");
        assert_eq!(validate(&legacy).unwrap().separate_command_keys,None);
        legacy["separateCommandKeys"]=json!(true);
        assert_eq!(validate(&legacy).unwrap().separate_command_keys,Some(true));
        legacy["separateCommandKeys"]=json!("true");
        assert!(validate(&legacy).is_err());
        assert_eq!(shortcut("RIGHT-CMD+a").unwrap(), "right-cmd+a");
        assert_eq!(shortcut("shift+left-cmd+a").unwrap(), "left-cmd+shift+a");
        for invalid in ["cmd+right-cmd+a", "left-cmd+right-cmd+a", "right-cmd+right-cmd+a"] {
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
        let fixture:Value=serde_json::from_str(include_str!("../../Tests/fixtures/search-quality.json")).unwrap();
        let commands:Vec<Command>=serde_json::from_value(fixture["commands"].clone()).unwrap();
        let index=SearchIndex::new(&commands);
        for case in fixture["cases"].as_array().unwrap() {
            let query=case["query"].as_str().unwrap();
            let got=index.search(query);
            assert_eq!(got.first().map(|i|commands[*i].id.as_str()),case["intended"].as_str(),"query {query}");
            assert_eq!(index.search(query),got,"determinism for {query}");
        }
    }
    #[test]
    fn single_typo_matches_independent_edit_neighborhood() {
        fn strings(n:usize)->Vec<String> {
            let mut all=vec![String::new()];let mut row=vec![String::new()];
            for _ in 0..n {row=row.iter().flat_map(|s|['a','b','Ω'].map(|c|format!("{s}{c}"))).collect();all.extend(row.clone());}all
        }
        let samples=strings(4);
        for a in &samples {
            let chars=a.chars().collect::<Vec<_>>();
            let mut neighbors=std::collections::HashSet::from([a.clone()]);
            for i in 0..=chars.len() {
                for c in ['a','b','Ω'] {let mut t=chars.clone();t.insert(i,c);neighbors.insert(t.iter().collect::<String>());}
                if i==chars.len() {continue;}
                let mut t=chars.clone();t.remove(i);neighbors.insert(t.iter().collect::<String>());
                for c in ['a','b','Ω'] {let mut t=chars.clone();t[i]=c;neighbors.insert(t.iter().collect::<String>());}
                if i+1<chars.len() {let mut t=chars.clone();t.swap(i,i+1);neighbors.insert(t.iter().collect::<String>());}
            }
            for b in &samples {assert_eq!(one_typo(a,b),neighbors.contains(b),"{a:?} vs {b:?}");}
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
