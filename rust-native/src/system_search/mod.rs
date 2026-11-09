//! Demand-only OS searches. Opaque session tokens never become persistent IDs.
//! Native handles stay in the backend; neither diagnostic logs nor configuration
//! receive browser addresses, window titles or menu paths.
pub mod apple_shortcuts;
#[cfg(any(test, feature = "test-tools"))]
pub mod fixture;
pub mod main_bridge;
pub mod native;
pub mod providers;
pub mod service;
#[cfg(test)]
mod tests;

use rustcast_core::Command;
use serde::{Deserialize, Serialize};
use std::{sync::Arc, time::Instant};

pub const EXPIRED: &str = "This result has expired. Search again.";
pub const CANCELLED: &str = "Search cancelled or timed out. Refresh to retry.";
pub const SAFARI_PERMISSION: &str =
    "Allow Safari access in System Settings → Privacy & Security → Automation, then refresh.";

#[derive(Clone)]
pub struct Ticket {
    pub deadline: Instant,
    pub live: crate::execution::Live,
}
impl Ticket {
    pub fn check(&self) -> Result<(), String> {
        if Instant::now() < self.deadline && (self.live)() {
            Ok(())
        } else {
            Err(CANCELLED.into())
        }
    }
    pub fn remaining(&self) -> Result<f64, String> {
        self.check()?;
        Ok(self
            .deadline
            .saturating_duration_since(Instant::now())
            .as_secs_f64()
            .max(0.001))
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Identity {
    pub pid: i32,
    pub uid: u32,
    pub seconds: u64,
    pub micros: u64,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Element(pub u64);
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Application {
    pub identity: Identity,
    pub name: String,
    pub bundle_identifier: Option<String>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Shortcut {
    pub keys: Vec<String>,
    pub label: String,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MenuInfo {
    pub application: String,
    pub path: String,
    pub title: String,
    pub shortcut: Option<Shortcut>,
    pub enabled: bool,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WindowInfo {
    pub application: String,
    pub title: String,
    pub minimized: Option<bool>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TabInfo {
    pub url: String,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Metadata {
    Menu { menu: MenuInfo },
    Window { window: WindowInfo },
    Tab { tab: TabInfo },
    Process { process: crate::processes::Resource },
}
#[derive(Clone, Debug, Default)]
pub struct Results {
    pub foreground: Option<Application>,
    pub commands: Vec<Command>,
    pub metadata: std::collections::HashMap<String, Metadata>,
}
impl Results {
    pub fn json(&self) -> serde_json::Value {
        serde_json::Value::Array(
            self.commands
                .iter()
                .map(|c| {
                    let mut row = serde_json::to_value(c).expect("Command serializes");
                    if let Some(m) = self.metadata.get(&c.id) {
                        if let (Some(row), Some(extra)) = (
                            row.as_object_mut(),
                            serde_json::to_value(m)
                                .ok()
                                .and_then(|v| v.as_object().cloned()),
                        ) {
                            row.extend(extra);
                        }
                    }
                    row
                })
                .collect(),
        )
    }
    pub fn truncate(&mut self, limit: usize) {
        self.commands.truncate(limit);
        self.metadata
            .retain(|id, _| self.commands.iter().any(|c| &c.id == id));
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scope {
    MainTabs,
    Tabs,
    Windows,
    Menus,
    Shortcuts,
    Files,
    Processes,
}
impl Scope {
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "commands" => Self::MainTabs,
            "tabs" => Self::Tabs,
            "windows" => Self::Windows,
            "menus" => Self::Menus,
            "shortcuts" => Self::Shortcuts,
            "files"=>Self::Files,
            "processes"=>Self::Processes,
            _ => return None,
        })
    }
    // Absolute request safety cap, distinct from the Window/Menu provider's
    // original private two-second traversal clock. In particular, Menu identity
    // validation precedes that private clock; caller expiry must not consume it.
    pub fn budget(self) -> std::time::Duration {
        std::time::Duration::from_secs(match self {
            Self::MainTabs => 1,
            Self::Files=>8,
            Self::Processes=>2,
            _ => 8,
        })
    }
    pub fn placeholder(self) -> &'static str {
        match self {
            Self::MainTabs => "Search apps, tabs or commands…",
            Self::Tabs => "Search Safari tabs…",
            Self::Windows => "Search windows…",
            Self::Menus => "Search app menu…",
            Self::Shortcuts => "Search Apple Shortcuts…",
            Self::Files=>"Search files…",
            Self::Processes=>"Search processes…",
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Tab {
    pub window: i32,
    pub index: i32,
    pub title: String,
    pub url: String,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MenuNode {
    pub element: Element,
    pub role: String,
    pub title: String,
    pub enabled: Option<bool>,
    pub character: Option<String>,
    pub glyph: Option<i64>,
    pub virtual_key: Option<i64>,
    pub modifiers: Option<i64>,
}
/// Each native message is capped by the request's remaining absolute deadline.
/// Fixtures implement this interface without reading any external application.
pub trait Platform: Send + Sync {
    fn files(&self,_query:&str,_ticket:&Ticket)->Result<Vec<Command>,String>{Err("File search is unavailable.".into())}
    fn processes(&self,_ticket:&Ticket)->Result<Vec<crate::processes::Resource>,String>{Err("Process search is unavailable.".into())}
    fn terminate(&self,_identity:Identity,_ticket:&Ticket)->Result<(),String>{Err("Force Quit is unavailable.".into())}
    fn identity(&self, pid: i32) -> Option<Identity>;
    fn trusted(&self) -> bool;
    fn frame(&self, _node: Element, _ticket: &Ticket) -> Result<rustcast_core::Frame, String> { Err("This window does not expose its frame.".into()) }
    fn resize(&self, _node: Element, _frame: rustcast_core::Frame, _ticket: &Ticket) -> Result<(), String> { Err("This window does not allow resizing.".into()) }
    fn position(&self, _node: Element, _frame: rustcast_core::Frame, _ticket: &Ticket) -> Result<(), String> { Err("This window does not allow moving.".into()) }
    fn applications(&self, ticket: &Ticket) -> Result<Vec<Application>, String>;
    fn application(&self, pid: i32, ticket: &Ticket) -> Result<Element, String>;
    fn pid(&self, node: Element) -> Option<i32>;
    fn string(&self, node: Element, key: &str, ticket: &Ticket) -> Result<Option<String>, String>;
    fn boolean(&self, node: Element, key: &str, ticket: &Ticket) -> Result<Option<bool>, String>;
    fn integer(&self, node: Element, key: &str, ticket: &Ticket) -> Result<Option<i64>, String>;
    fn elements(
        &self,
        node: Element,
        key: &str,
        limit: usize,
        unsupported_empty: bool,
        ticket: &Ticket,
    ) -> Result<Vec<Element>, String>;
    fn element(&self, node: Element, key: &str, ticket: &Ticket)
        -> Result<Option<Element>, String>;
    fn same(&self, a: Element, b: Element) -> bool;
    fn restore(&self, node: Element, ticket: &Ticket) -> Result<(), String>;
    fn action(&self, node: Element, action: &str, ticket: &Ticket) -> Result<(), String>;
    fn safari(&self, ticket: &Ticket) -> Result<Identity, String>;
    fn safari_windows(&self, browser: Identity, ticket: &Ticket) -> Result<Vec<i32>, String>;
    fn safari_tabs(
        &self,
        browser: Identity,
        window: i32,
        ticket: &Ticket,
    ) -> Result<Vec<Tab>, String>;
    fn safari_select(&self, browser: Identity, tab: &Tab, ticket: &Ticket) -> Result<(), String>;
    fn safari_selected(
        &self,
        browser: Identity,
        tab: &Tab,
        ticket: &Ticket,
    ) -> Result<bool, String>;
    fn safari_front_window(
        &self,
        browser: Identity,
        window: i32,
        ticket: &Ticket,
    ) -> Result<(), String>;
    fn shortcuts(&self, ticket: &Ticket) -> Result<Vec<Command>, String>;
    fn run_shortcut(&self, id: &str, ticket: &Ticket) -> Result<(), String> {
        apple_shortcuts::run(id, ticket)
    }
    fn safari_consent(&self, ticket: &Ticket) -> Result<(), String>;
}
pub type Activate = Arc<dyn Fn(Identity, &Ticket) -> Result<(), String> + Send + Sync>;
pub type Prepare = Arc<dyn Fn(&Ticket) -> Result<(), String> + Send + Sync>;

pub fn command(id: String, title: String, enabled: Option<bool>) -> Command {
    serde_json::from_value(
        serde_json::json!({"id":id,"title":title,"kind":"builtin","target":id,"enabled":enabled}),
    )
    .expect("fixed command")
}
pub fn matches(text: &str, query: &str) -> bool {
    // Foundation folding preserves the shipped case/diacritic-insensitive Safari
    // search without normalizing or retaining the user's query in diagnostics.
    let fold = |s: &str| {
        use objc2_foundation::NSString;
        let s = NSString::from_str(s);
        unsafe {
            let folded: objc2::rc::Retained<NSString> = objc2::msg_send![&*s, stringByFoldingWithOptions: 385usize, locale: std::ptr::null::<objc2_foundation::NSLocale>()];
            folded.to_string().to_lowercase()
        }
    };
    let text = fold(text);
    text.contains(&fold(query))
}

pub fn matches_words(text: &str, query: &str) -> bool {
    query.split_whitespace().all(|word| matches(text, word))
}

/// Source CommandEditor.swift entry points; saved IDs/targets override these.
pub fn commands(saved: &[Command]) -> Vec<Command> {
    [
        ("builtin.window-search","Search Windows","switch-windows",vec!["windows","switch-windows"]),
        ("builtin.menu-actions","Search Menu Actions","search-menu-items",vec!["menu-actions","menu-items"]),
        ("builtin.safari-tabs","Search Safari Tabs","search-safari-tabs",vec!["tabs","safari-tabs","tabbladen"]),
        ("builtin.apple-shortcuts","Search Apple Shortcuts","apple-shortcuts",vec!["apple-shortcuts","opdrachten"]),
    ].into_iter().filter(|(id,_,target,_)|!saved.iter().any(|s|s.id==*id || (s.kind=="builtin" && s.target==*target))).map(|(id,title,target,aliases)|serde_json::from_value(serde_json::json!({"id":id,"title":title,"kind":"builtin","target":target,"aliases":aliases})).expect("fixed builtin")).collect()
}
