//! Settings navigation belongs to its window; profile/configuration stays shared.
use rustcast_core::Command;
use std::collections::BTreeSet;
#[derive(Clone, Debug, PartialEq)]
pub struct Navigation {
    pub tab: String,
    pub filter: String,
    pub collapsed: BTreeSet<String>,
}
impl Default for Navigation {
    fn default() -> Self {
        Self {
            tab: "commands".into(),
            filter: "all".into(),
            collapsed: BTreeSet::new(),
        }
    }
}
#[derive(Clone, Debug, PartialEq)]
pub struct Info {
    pub tab: String,
    pub filter: String,
    pub groups: Vec<Group>,
}
#[derive(Clone, Debug, PartialEq)]
pub struct Group {
    pub title: String,
    pub count: usize,
    pub collapsed: bool,
}
impl Navigation {
    pub fn matches(&self, c: &Command) -> bool {
        (match self.tab.as_str() {
            "snippets" => c.kind == "snippet",
            "quicklinks" => ["url", "path"].contains(&c.kind.as_str()),
            _ => true,
        }) && (match self.filter.as_str() {
            "all" => true,
            "disabled" => c.enabled == Some(false),
            kind => c.kind == kind,
        })
    }
    pub fn visible(&self, c: &Command, query: &str) -> bool {
        !query.is_empty() || !self.collapsed.contains(group(c))
    }
}
pub fn group(c: &Command) -> &'static str {
    match c.kind.as_str() {
        "app" => "Applications",
        "window" => "Window Management",
        "snippet" => "Snippets",
        "url" | "path" => "Quicklinks",
        _ => {
            let key = format!("{} {} {}", c.id, c.title, c.target).to_lowercase();
            if key.contains("pomodoro") {
                "Pomodoro"
            } else if key.contains("dictate") {
                "Dictate"
            } else if key.contains("ai-usage") || key.contains("ai usage") {
                "AI Usage"
            } else if key.contains("task") {
                "Tasks"
            } else if c.kind == "builtin" {
                "Ruycast"
            } else {
                "Scripts & Actions"
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::session::{fixture, Session};
    #[test]
    fn all_records_and_disabled_remain_available_after_reload() {
        let mut commands = fixture(5000);
        commands[4700].enabled = Some(false);
        let mut s = Session::management(commands.clone());
        assert_eq!(s.row_count(), 5000);
        s.set_query("alias4700");
        assert_eq!(s.selected().unwrap().enabled, Some(false));
        s.replace_commands(commands);
        assert_eq!(s.selected().unwrap().id, "fixture.4700");
        assert_eq!(s.row_count(), 1);
    }
    #[test]
    fn scopes_groups_and_search_preserve_identity_without_replacement() {
        let mut c = fixture(4);
        c[0].kind = "snippet".into();
        let mut s = Session::management(c.clone());
        s.navigate("tab", "snippets").unwrap();
        assert!(s.selected().is_none());
        assert_eq!(s.row_count(), 1);
        s.navigate("tab", "commands").unwrap();
        let rev = s.results_revision();
        s.navigate("group", "Quicklinks").unwrap();
        assert!(s.results_revision() > rev);
        assert_eq!(s.row_count(), 1);
        s.set_query("alias1");
        assert_eq!(s.row_count(), 1);
        s.set_query("");
        assert_eq!(s.row_count(), 1);
        s.navigate("filter", "disabled").unwrap();
        assert_eq!(s.row_count(), 0);
        assert!(s.navigate("tab", "unknown").is_err());
    }
    #[test]
    fn grouped_snapshot_has_command_selection_and_shared_row_storage() {
        let s = Session::management(fixture(1000));
        let view = crate::connected::View::from_session(&s, 0, 0, None, String::new());
        assert!(view.rows[0].is_group);
        assert!(!view.rows[view.selection.unwrap()].is_group);
        let copy = view.clone();
        assert!(std::sync::Arc::ptr_eq(&view.rows, &copy.rows));
        assert_eq!(view.rows.len(), 1001);
    }
}
