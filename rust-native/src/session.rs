//! Rust-owned launcher state. No platform handles or persistent effects.
use crate::catalog::Catalog;
use rustcast_core::Command;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LauncherScope {
    Snippets,
    Quicklinks,
}
impl LauncherScope {
    pub fn title(self) -> &'static str {
        match self { Self::Snippets => "Snippets", Self::Quicklinks => "Quicklinks" }
    }
    pub fn placeholder(self) -> &'static str {
        match self { Self::Snippets => "Search snippets…", Self::Quicklinks => "Search quicklinks…" }
    }
    pub fn parse(mode: &str) -> Option<Self> {
        match mode {
            "snippets" => Some(Self::Snippets),
            "quicklinks" => Some(Self::Quicklinks),
            _ => None,
        }
    }
    fn matches(self, command: &Command) -> bool {
        match self {
            Self::Snippets => command.kind == "snippet",
            Self::Quicklinks => matches!(command.kind.as_str(), "url" | "path"),
        }
    }
}

pub struct Session {
    pub navigation_revision: u64,
    pub clipboard_mode: bool,
    pub launcher_scope: Option<LauncherScope>,
    pub catalog: Catalog,
    pub library: Option<crate::library::Navigation>,
    results: Vec<usize>,
    external: Option<Vec<Command>>,
    results_revision: u64,
    pub query: String,
    pub selection: Option<usize>,
    pub selection_scroll_serial: u64,
    pub visible: bool,
    pub actions_open: bool,
    pub activated_id: Option<String>,
}

impl Session {
    pub fn new(commands: Vec<Command>) -> Self {
        let mut session = Self {
            navigation_revision: 0,
            clipboard_mode: false,
            launcher_scope: None,
            catalog: Catalog::new(commands),
            library: None,
            results: Vec::new(),
            external: None,
            results_revision: 0,
            query: String::new(),
            selection: None,
            selection_scroll_serial: 0,
            visible: false,
            actions_open: false,
            activated_id: None,
        };
        session.set_query("");
        session
    }
    pub fn management(commands: Vec<Command>) -> Self {
        let mut session = Self::new(vec![]);
        session.catalog = Catalog::with_disabled(commands, true);
        session.library = Some(Default::default());
        session.set_query("");
        session
    }
    pub fn library_info(&self) -> Option<crate::library::Info> {
        let nav = self.library.as_ref()?;
        let mut groups = std::collections::BTreeMap::<String, usize>::new();
        for index in self.catalog.search(&self.query, 0) {
            let c = self.catalog.command(index);
            if nav.matches(c) {
                *groups.entry(crate::library::group(c).into()).or_default() += 1;
            }
        }
        Some(crate::library::Info {
            tab: nav.tab.clone(),
            filter: nav.filter.clone(),
            groups: groups
                .into_iter()
                .map(|(title, count)| crate::library::Group {
                    collapsed: nav.collapsed.contains(&title),
                    title,
                    count,
                })
                .collect(),
        })
    }
    pub fn navigate(&mut self, kind: &str, value: &str) -> Result<(), String> {
        let known_group = self
            .library_info()
            .is_some_and(|info| info.groups.iter().any(|g| g.title == value));
        let nav = self.library.as_mut().ok_or("Library is unavailable")?;
        match kind {
            "tab" => {
                if !["commands", "snippets", "quicklinks", "dashboard", "general", "advanced", "clipboard"].contains(&value) {
                    return Err("Settings tab is not yet available.".into());
                }
                if nav.tab != value {
                    self.navigation_revision=self.navigation_revision.checked_add(1).ok_or("Settings navigation sequence exhausted.")?;
                }
                nav.tab = value.into();
                nav.filter = "all".into();
                self.query.clear();
                self.selection = None;
            }
            "filter" => {
                if !["all", "app", "window", "script", "builtin", "disabled"].contains(&value) {
                    return Err("Unknown command filter".into());
                }
                nav.filter = value.into();
            }
            "group" => {
                if !known_group {
                    return Err("Group changed. Try again.".into());
                }
                if !nav.collapsed.remove(value) {
                    nav.collapsed.insert(value.into());
                }
            }
            _ => return Err("Unknown library action".into()),
        }
        self.refresh_preserving_selection();
        Ok(())
    }
    fn search(&self) -> Vec<usize> {
        if self.clipboard_mode { return (0..self.catalog.len()).filter(|&i|self.query.is_empty()||crate::clipboard_history::matches(&self.catalog.command(i).target,&self.query)).collect(); }
        if self.library.as_ref().is_some_and(|n| ["dashboard","general","advanced","clipboard"].contains(&n.tab.as_str())) { return vec![]; }
        let mut rows = self
            .catalog
            .search(&self.query, if self.library.is_some() || self.launcher_scope.is_some() { 0 } else { 100 });
        if let Some(scope) = self.launcher_scope {
            rows.retain(|&i| scope.matches(self.catalog.command(i)));
            rows.truncate(100);
        }
        if let Some(nav) = &self.library {
            rows.retain(|&i| {
                let c = self.catalog.command(i);
                nav.matches(c) && nav.visible(c, &self.query)
            });
            rows.sort_by_key(|&i| crate::library::group(self.catalog.command(i)));
        }
        rows
    }
    pub fn set_query(&mut self, query: &str) {
        self.external = None;
        self.query.clear();
        self.query.push_str(query);
        self.results = self.search();
        self.results_revision = self.results_revision.wrapping_add(1);
        self.selection = (!self.results.is_empty()).then_some(0);
        self.actions_open = false;
        self.activated_id = None;
    }
    pub fn replace_commands(&mut self, commands: Vec<Command>) {
        let selected = self.selected().map(|c| c.id.clone());
        if self.catalog.replace(commands) {
            self.refresh_with_selection(selected);
        }
    }
    pub fn refresh_preserving_selection(&mut self) {
        let selected = self.selected().map(|c| c.id.clone());
        self.refresh_with_selection(selected);
    }
    fn refresh_with_selection(&mut self, selected: Option<String>) {
        self.results = self.search();
        self.results_revision = self.results_revision.wrapping_add(1);
        self.selection = selected.and_then(|id| {
            self.results
                .iter()
                .position(|&i| self.catalog.command(i).id == id)
        });
        if self.selection.is_none() {
            self.actions_open = false;
        }
        self.activated_id = None;
    }
    pub fn set_external_results(&mut self, rows: Vec<Command>, preserve: bool) {
        let selected = preserve.then(|| self.selected().map(|c|c.id.clone())).flatten();
        self.external = Some(rows);
        self.results_revision = self.results_revision.wrapping_add(1);
        self.selection = if preserve { selected.and_then(|id|self.rows().position(|c|c.id==id)) } else { (self.row_count()>0).then_some(0) };
        self.activated_id=None;
    }
    pub fn rows(&self) -> impl Iterator<Item = &Command> {
        (0..self.row_count()).filter_map(|i|self.row(i))
    }
    pub fn results_revision(&self) -> u64 {
        self.results_revision
    }
    pub fn row_count(&self) -> usize {
        self.external.as_ref().map_or(self.results.len(),Vec::len)
    }
    pub fn row(&self, index: usize) -> Option<&Command> {
        if let Some(rows)=&self.external {rows.get(index)} else {self.results.get(index).map(|&i| self.catalog.command(i))}
    }
    pub fn select_row(&mut self, index: Option<usize>) {
        self.selection = index.filter(|&i| i < self.row_count());
        self.activated_id = None;
        self.actions_open = false;
    }
    pub fn selected(&self) -> Option<&Command> {
        self.selection.and_then(|i|self.row(i))
    }
    pub fn move_selection(&mut self, delta: isize) {
        if self.row_count()==0 {
            self.selection = None;
            return;
        }
        self.selection_scroll_serial = self.selection_scroll_serial.wrapping_add(1);
        let next = match self.selection {
            Some(index) => (index as isize + delta).clamp(0, self.row_count() as isize - 1),
            None if delta < 0 => self.row_count() as isize - 1,
            None => 0,
        };
        self.selection = Some(next as usize);
    }
    /// Native input methods own Return/arrows/Escape while composing text.
    /// Prototype activation records intent only: never execute a user action.
    pub fn key(&mut self, selector: &str, marked_text: bool) -> bool {
        if marked_text {
            return false;
        }
        match selector {
            "moveDown:" => self.move_selection(1),
            "moveUp:" => self.move_selection(-1),
            "insertNewline:" => {
                self.activated_id = self.selected().map(|c| c.id.clone());
                if self.activated_id.is_some() {
                    self.visible = false;
                }
            }
            "cancelOperation:" => {
                if self.actions_open {
                    self.actions_open = false;
                } else {
                    self.visible = false;
                }
            }
            _ => return false,
        }
        true
    }
}

pub fn fixture(size: usize) -> Vec<Command> {
    (0..size)
        .map(|i| Command {
            id: format!("fixture.{i}"),
            title: format!(
                "{} {i}",
                [
                    "Microsoft Teams",
                    "Open Notes",
                    "Left Half",
                    "Project Dashboard"
                ][i % 4]
            ),
            kind: "url".into(),
            target: format!("https://example.com/{i}"),
            aliases: vec![format!("alias{i}")],
            arguments: vec![],
            hotkey: None,
            imported_hotkey: None,
            timeout: None,
            input_prompt: None,
            keyword: None,
            application_id: None,
            enabled: None,
            snippet_output: None,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustcast_core::SearchIndex;
    #[test]
    fn scoped_launcher_filters_before_limit_preserves_ranking_and_excludes_disabled() {
        let mut commands = fixture(240);
        for (i, command) in commands.iter_mut().enumerate() {
            command.kind = if i < 120 { "app" } else if i % 3 == 0 { "snippet" } else if i % 3 == 1 { "url" } else { "path" }.into();
            command.aliases = vec!["résumé".into()];
        }
        commands[120].enabled = Some(false);
        let mut session = Session::new(commands);
        session.catalog.set_usage(std::collections::HashMap::from([("fixture.126".into(), 50)]));
        for scope in [LauncherScope::Snippets, LauncherScope::Quicklinks] {
            session.launcher_scope = Some(scope);
            for query in ["", "résumé", "re\u{301}sume\u{301}"] {
                let expected: Vec<_> = session.catalog.search(query, 0).into_iter()
                    .filter(|&i| scope.matches(session.catalog.command(i))).take(100).collect();
                session.set_query(query);
                assert_eq!(session.results, expected);
                if query.is_empty() { assert!(!session.results.is_empty()); }
                assert!(session.library.is_none());
                assert!(!session.results.iter().any(|&i| session.catalog.command(i).id == "fixture.120"));
            }
        }
        session.launcher_scope = None;
        session.set_query("");
        assert_eq!(session.results.len(), 100);
    }
    #[test]
    fn uses_exact_existing_kernel_order_including_unicode_and_empty() {
        let commands = fixture(500);
        let index = SearchIndex::new(&commands);
        let mut session = Session::new(commands);
        for query in ["", "m", "alias42", "notes", "missing Ω", "fnider", "a\0b"] {
            session.set_query(query);
            assert_eq!(
                session.results,
                index
                    .search(query)
                    .into_iter()
                    .take(100)
                    .collect::<Vec<_>>()
            );
        }
        session.set_query("alias42");
        assert_eq!(session.selected().unwrap().id, "fixture.42");
    }
    #[test]
    fn ime_must_not_navigate_activate_or_close() {
        let mut session = Session::new(fixture(8));
        session.visible = true;
        for key in ["insertNewline:", "cancelOperation:", "moveDown:"] {
            assert!(!session.key(key, true));
            assert!(session.visible);
            assert!(session.activated_id.is_none());
            assert_eq!(session.selection, Some(0));
        }
        assert!(session.key("moveDown:", false));
        assert_eq!(session.selection, Some(1));
        assert!(session.key("insertNewline:", false));
        assert!(!session.visible);
        assert!(session.activated_id.is_some());
    }
    #[test]
    fn empty_results_have_no_accidental_execution_and_navigation_is_bounded() {
        let mut session = Session::new(fixture(3));
        for _ in 0..20 {
            session.move_selection(1);
        }
        assert_eq!(session.selection, Some(2));
        session.set_query("not-existing-Ω");
        session.key("insertNewline:", false);
        assert!(session.selection.is_none());
        assert!(session.activated_id.is_none());
    }
    #[test]
    fn fresh_query_and_explicit_selection_clear_stale_activation() {
        let mut session = Session::new(fixture(120));
        session.key("insertNewline:", false);
        assert_eq!(session.activated_id.as_deref(), Some("fixture.0"));
        session.set_query("alias42");
        assert!(session.activated_id.is_none());
        let revision = session.results_revision();
        session.move_selection(1);
        assert_eq!(session.results_revision(), revision);
        session.select_row(Some(usize::MAX));
        assert!(session.selected().is_none());
        session.key("insertNewline:", false);
        assert!(session.activated_id.is_none());
        session.move_selection(1);
        assert_eq!(session.selection, Some(0));
    }
    #[test]
    fn disabled_commands_never_resolve_or_activate() {
        let mut commands = fixture(8);
        commands[2].enabled = Some(false);
        let mut session = Session::new(commands);
        session.set_query("alias2");
        assert!(!session.rows().any(|c| c.id == "fixture.2"));
    }
    #[test]
    fn catalog_reload_preserves_selected_identity_and_does_not_choose_replacement() {
        let mut session = Session::new(fixture(12));
        session.set_query("microsoft");
        session.move_selection(1);
        let selected = session.selected().unwrap().id.clone();
        session.actions_open = true;
        let mut commands = fixture(12);
        commands.reverse();
        session.replace_commands(commands.clone());
        assert_eq!(session.selected().unwrap().id, selected);
        assert!(session.actions_open);
        commands.retain(|c| c.id != selected);
        session.replace_commands(commands);
        assert!(session.selected().is_none());
        assert!(!session.actions_open);
        session.key("insertNewline:", false);
        assert!(session.activated_id.is_none());
    }
}
