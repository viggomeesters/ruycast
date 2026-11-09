//! Settings share the configuration writer and its keyboard staging transaction.
use serde::{Deserialize, Serialize};
use serde_json::Value;
/// The frozen SettingsTheme colors, shared by native management surfaces.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Palette {
    pub background: [f64; 3],
    pub ink: [f64; 3],
    pub muted: [f64; 3],
    pub accent: [f64; 3],
}
impl Palette {
    pub fn new(dark: bool, graphite: bool) -> Self {
        Self {
            background: if graphite {
                [0.15; 3]
            } else if dark {
                [0.10, 0.12, 0.17]
            } else {
                [0.98, 0.97, 0.94]
            },
            ink: if dark {
                [0.96, 0.95, 0.91]
            } else {
                [0.18, 0.20, 0.25]
            },
            muted: if dark {
                [0.68, 0.71, 0.77]
            } else {
                [0.42, 0.44, 0.48]
            },
            accent: if dark {
                [1., 0.66, 0.35]
            } else {
                [0.64, 0.30, 0.08]
            },
        }
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ClipboardStatus {
    pub count: usize,
    pub paused: bool,
    pub pause_error: Option<String>,
    pub cleanup_error: Option<String>,
    pub warning: Option<String>,
}
impl ClipboardStatus {
    pub fn error(&self) -> Option<&str> {
        self.pause_error
            .as_deref()
            .or(self.cleanup_error.as_deref())
            .or(self.warning.as_deref())
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Surface {
    pub navigation_revision: u64,
    pub tab: String,
    pub revision: u64,
    pub epoch: u64,
    pub dismissal: u64,
    pub configuration: Value,
    pub directory: String,
    pub path: String,
    pub busy: bool,
    pub message: String,
    pub clipboard: Option<ClipboardStatus>,
}
#[derive(Clone, Debug)]
pub enum Operation {
    ClipboardPause(bool),
    Set { key: String, value: Value },
    Reload,
    Directory(String),
    Feedback(String),
}
/// Only these fields are writable from General. Preserve unknown fields and
/// commands from the latest accepted snapshot rather than serializing Config.
pub fn update(
    store: &mut crate::profile::ConfigStore,
    key: &str,
    value: Value,
) -> Result<Option<String>, String> {
    match key {
        "launcherHotkey" | "clipboardHotkey" if value.is_string() => {}
        "hyperKeyEnabled" | "separateCommandKeys" | "menuBarVisible" if value.is_boolean() => {}
        "clipboard" if value.is_object() => {}
        "theme"
            if value
                .as_str()
                .is_some_and(|v| ["system", "light", "dark", "graphite"].contains(&v)) => {}
        _ => return Err("Invalid setting.".into()),
    }
    let mut document: Value =
        serde_json::from_slice(store.snapshot()).map_err(|_| "Configuration could not be read.")?;
    document[key] = value;
    Ok(store.save(&document)?.warning)
}
