//! Fixtures own a Session. Live views own immutable presentation snapshots;
//! only the background runtime owns a live mutable Session/HostState.
use rustcast_core::Command;
use ruycast_native::{
    connected::{Action, Connection, View},
    presentation,
    session::Session,
};
use std::sync::Arc;

pub struct RowRef<'a> {
    pub id: &'a str,
    pub icon: presentation::Icon,
    pub detail: Option<&'a str>,
    pub keyword: Option<&'a str>,
    pub title: &'a str,
    pub kind: &'a str,
    pub subtitle: Option<&'a str>,
    pub system: bool,
    pub app_shortcut: Option<&'a ruycast_native::system_search::Shortcut>,
    pub hotkey: Option<&'a str>,
    pub action: &'static str,
    pub is_group: bool,
    pub aliases: &'a [String],
    pub enabled: bool,
}
// Only one palette and one Settings model exist. Keep their snapshots in place:
// boxing both variants merely to cross clippy's size heuristic adds allocations
// and indirection. Neither model is moved or copied by inline text editing.
#[allow(clippy::large_enum_variant)]
pub enum Model {
    Fixture(Session),
    Live {
        view: View,
        connection: Arc<Connection>,
    },
}
impl From<Session> for Model {
    fn from(mut s: Session) -> Self {
        s.visible = true;
        Self::Fixture(s)
    }
}
impl Model {
    pub fn utility_scope(&self)->Option<String>{match self{Self::Live{view,..}=>view.utility_scope.clone(),_=>None}}
    pub fn utility_busy(&self)->bool{matches!(self,Self::Live{view,..} if view.utility_busy)}
    pub fn results_heading(&self)->String{if let Self::Live{view,..}=self{use ruycast_native::system_search::Scope;match view.system_scope{Some(Scope::Processes)=>return format!("Processes · {}",self.row_count()),Some(Scope::Menus)=>return "Menu Actions".into(),Some(Scope::Windows)=>return "Windows".into(),Some(Scope::Tabs)=>return "Safari Tabs".into(),Some(Scope::Shortcuts)=>return "Apple Shortcuts".into(),Some(Scope::MainTabs) if view.rows.first().is_some_and(|r|matches!(view.system_metadata.get(&r.id),Some(ruycast_native::system_search::Metadata::Tab{..})))=>return "Safari Tabs".into(),_=>{}}}
        if self.query().is_empty(){"Suggestions".into()}else{"Results".into()}}
    pub fn monitor(&self)->Option<std::sync::Arc<ruycast_native::monitor::Surface>>{match self{Self::Live{view,..}=>view.monitor.clone(),_=>None}}
    pub fn process_state(&self)->Option<(ruycast_native::processes::Sort,u64,u64)>{match self{Self::Live{view,..} if view.visible &&view.system_scope==Some(ruycast_native::system_search::Scope::Processes)&&view.editor.is_none()&&view.script_result.is_none()=>Some((view.process_sort,view.epoch,view.dismissal)),_=>None}}
    pub fn os_connection(&self)->Option<std::sync::Arc<Connection>>{match self{Self::Live{connection,..}=>Some(connection.clone()),_=>None}}
    pub fn dashboard(&self) -> Option<std::sync::Arc<ruycast_native::analytics_dashboard::Surface>> {
        match self { Self::Live{view,..} => view.dashboard.clone(), _=>None }
    }
    pub fn dashboard_connection(&self) -> Option<std::sync::Arc<Connection>> {
        match self {Self::Live{connection,..} if connection.is_management=>Some(connection.clone()),_=>None}
    }
    pub fn dashboard_request(&self) -> Option<(u64,u64,u64)> {
        match self {Self::Live{view,..} if view.visible=>view.dashboard_request,_=>None}
    }
    pub fn script_result(&self) -> Option<&ruycast_native::script_session::Surface> {
        match self {
            Self::Live { view, .. } => view.script_result.as_ref(),
            _ => None,
        }
    }
    pub fn result_action(&self, token: u64, name: &str, id: Option<String>) {
        if let Self::Live { view, connection } = self {
            connection.action(Action::Result {
                token,
                name: name.into(),
                id,
                epoch: if ["choose", "up", "down"].contains(&name) {
                    connection.latest_epoch()
                } else {
                    view.epoch
                },
            });
        }
    }
    pub fn mark_result_closing(&mut self, token: u64) {
        if let Self::Live { view, .. } = self {
            if let Some(r) = view.script_result.as_mut().filter(|r| r.token == token) {
                r.busy = true;
                r.detail_visible = false;
                r.query = Some(String::new());
                r.rows.clear();
                r.selection = None;
                r.error.clear();
                view.rows = Arc::new(vec![]);
                view.selection = None;
            }
        }
    }
    pub fn result_back(&mut self) {
        if let Some(r) = self.script_result().cloned() {
            let name = if r.detail_visible && r.has_choices {
                "choices"
            } else {
                "close"
            };
            self.result_action(r.token, name, None);
            if name == "close" {
                self.mark_result_closing(r.token);
            }
        }
    }
    pub fn recording_owner(&self) -> Option<u64> {
        match self {
            Self::Live { view, .. } if view.visible => view
                .editor
                .as_ref()
                .filter(|e| {
                    !e.saving
                        && matches!(
                            e.mode,
                            ruycast_native::actions::Mode::Edit
                                | ruycast_native::actions::Mode::Hotkey
                        )
                })
                .map(|e| e.token),
            _ => None,
        }
    }
    pub fn editor_saving(&self) -> bool {
        matches!(self, Self::Live {view, ..} if view.editor.as_ref().is_some_and(|e| e.saving))
    }
    pub fn run_inline(&self) -> bool {
        matches!(self,Self::Live{view,..} if view.script_result.is_none() && view.editor.as_ref().is_some_and(|e|e.run_inline))
    }
    pub fn input_menu_open(&self) -> bool {
        matches!(self,Self::Live{view,..} if view.actions_open && view.editor.as_ref().is_some_and(|e|e.run_inline))
    }
    pub fn execution_running(&self) -> bool {
        matches!(self,Self::Live{view,..} if view.execution_running)
    }
    pub fn live(connection: Arc<Connection>) -> Self {
        Self::Live {
            view: connection.view(),
            connection,
        }
    }
    pub fn management(&self) -> bool {
        matches!(self,Self::Live{connection,..} if connection.is_management)
    }
    pub fn library_binding_stamp(&self) -> Option<(u64, u64, u64)> {
        match self {
            Self::Live { view, connection } if connection.is_management => {
                Some((view.catalog_revision, view.epoch, view.dismissal))
            }
            _ => None,
        }
    }
    pub fn library_info(&self) -> Option<&ruycast_native::library::Info> {
        match self {
            Self::Live { view, .. } => view.library.as_ref(),
            _ => None,
        }
    }
    pub fn show_serial(&self) -> u64 {
        match self {
            Self::Live { view, .. } => view.show_serial,
            _ => 0,
        }
    }
    pub fn library_action(
        &mut self,
        kind: &str,
        value: &str,
        aliases: Option<String>,
    ) -> Option<(u64, u64)> {
        if self.editor_saving() {
            return None;
        }
        if !self.visible() {
            return None;
        }
        let editor = self.surface().editor;
        if let Some(editor) = editor.as_ref().filter(|e| e.inline) {
            if editor.mode == ruycast_native::actions::Mode::Alias && aliases.is_none() {
                return None;
            }
            if let Self::Live { view, connection } = self {
                return connection.library_transition(
                    kind.into(),
                    value.into(),
                    view.catalog_revision,
                    Some((editor.token, aliases, None)),
                );
            }
        }
        if kind == "group" && editor.is_some() {
            return None;
        }
        self.cancel_editor();
        if let Self::Live { view, connection } = self {
            connection.library_transition(kind.into(), value.into(), view.catalog_revision, None);
        }
        None
    }
    pub fn inline_global(&self, name: &str, aliases: Option<String>) -> Option<(u64, u64)> {
        if self.editor_saving() {
            return None;
        }
        if let Self::Live { view, connection } = self {
            if !connection.is_management
                || !view.visible
                || connection.latest_dismissal() != view.dismissal
            {
                return None;
            }
            let previous = match view.editor.as_ref() {
                Some(e) if e.inline => Some((e.token, aliases, None)),
                _ => return None,
            };
            return connection.inline_global(name.into(), previous);
        }
        None
    }
    pub fn begin_inline(
        &self,
        target: ruycast_native::actions::Target,
        dismissal: u64,
        mode: ruycast_native::actions::Mode,
        aliases: Option<String>,
    ) -> Option<(u64, u64)> {
        if self.editor_saving() {
            return None;
        }
        let Self::Live { view, connection } = self else {
            return None;
        };
        if !connection.is_management
            || !view.visible
            || view.actions_open
            || view.dismissal != dismissal
            || connection.latest_dismissal() != dismissal
            || view.epoch != target.epoch
            || connection.latest_epoch() != target.epoch
            || view.catalog_revision != target.catalog_revision
            || !view.rows.iter().any(|r| !r.is_group && r.id == target.id)
        {
            return None;
        }
        let previous = match view.editor.as_ref() {
            Some(editor) if editor.inline => Some((editor.token, aliases, None)),
            Some(_) => return None,
            None => None,
        };
        connection.begin_inline(target, mode, previous)
    }
    pub fn library_enabled(
        &self,
        target: ruycast_native::actions::Target,
        dismissal: u64,
        expected: bool,
        enabled: bool,
        aliases: Option<String>,
    ) -> Option<(u64, u64)> {
        if self.editor_saving() {
            return None;
        }
        if let Self::Live { view, connection } = self {
            if !connection.is_management
                || !view.visible
                || view.editor.as_ref().is_some_and(|e| !e.inline)
                || view.actions_open
                || view.dismissal != dismissal
                || connection.latest_dismissal() != dismissal
                || view.catalog_revision != target.catalog_revision
                || connection.latest_epoch() != target.epoch
                || view.epoch != target.epoch
                || !view
                    .rows
                    .iter()
                    .any(|r| !r.is_group && r.id == target.id && r.enabled == expected)
            {
                return None;
            }
            let previous = view.editor.as_ref().map(|e| (e.token, aliases, None));
            return connection.set_enabled(target, expected, enabled, previous);
        }
        None
    }
    pub fn apply(&mut self, next: View) {
        if let Self::Live { view, .. } = self {
            *view = next;
        }
    }
    pub fn epoch(&self)->u64{match self{Self::Live{view,..}=>view.epoch,_=>0}}
    pub fn clipboard(&self)->Option<&ruycast_native::clipboard_history::Surface>{match self{Self::Live{view,..}=>view.clipboard.as_ref(),_=>None}}
    pub fn clipboard_action(&self,operation:&str,id:Option<String>,revision:u64,epoch:u64,dismissal:u64){if let Self::Live{connection,..}=self{connection.action(Action::Clipboard{operation:operation.into(),id,revision,epoch,dismissal});}}
    pub fn surface(&self) -> crate::native_surface::SurfaceData {
        match self {
            Self::Fixture(s) => crate::native_surface::SurfaceData {
                visible: s.visible,
                actions_open: s.actions_open,
                items: ruycast_native::actions::items(s.selected(), true),
                title: s.selected().map(|c| c.title.clone()).unwrap_or_default(),
                separate: true,
                ..Default::default()
            },
            Self::Live { view, connection } => crate::native_surface::SurfaceData {
                launcher_scope: view.launcher_scope,
                system_scope: view.system_scope,
                system_busy: view.system_busy,
                app_choices: view.app_choices.clone(),
                app_choices_pending: view.app_choices_pending,
                visible: view.visible,
                execution_running: view.execution_running,
                actions_open: view.actions_open,
                action_selection: view.action_selection,
                items: view.action_items.clone(),
                title: if view.clipboard.is_some(){String::new()}else{view
                    .editor
                    .as_ref()
                    .filter(|e| e.run_inline)
                    .map(|e| e.title.clone())
                    .unwrap_or_else(|| self.selected().map(|c| c.title.into()).unwrap_or_default())},
                menu_token: view.menu_token,
                editor: view.editor.clone(),
                separate: view.separate_command_keys,
                allow_hidden: connection.hidden_tests,
            },
        }
    }
    pub fn toggle_actions(&mut self) {
        if self.script_result().is_some() || self.dashboard().is_some() {
            return;
        }
        match self {
            Self::Fixture(s) => s.actions_open = !s.actions_open,
            Self::Live { connection, .. } => connection.action(Action::ToggleActions),
        }
    }
    pub fn launcher_back(&self) {
        if let Self::Live { view, connection } = self {
            if let Some(scope) = view.launcher_scope { connection.action(Action::LauncherBack(scope)); }
        }
    }
    pub fn named(&mut self, name: &str, menu: bool, index: Option<usize>) {
        if self.script_result().is_some() || self.dashboard().is_some() {
            return;
        }
        if let Self::Live { view, connection } = self {
            let target = if ruycast_native::actions::global(name) {
                None
            } else if menu {
                view.action_target.clone()
            } else {
                index
                    .and_then(|i| view.rows.get(i))
                    .filter(|r| !r.is_group)
                    .map(|row| ruycast_native::actions::Target {
                        id: row.id.clone(),
                        catalog_revision: view.catalog_revision,
                        epoch: view.epoch,
                    })
            };
            if target.is_none() && !ruycast_native::actions::global(name) {
                return;
            }
            connection.action(Action::Named {
                name: name.into(),
                target,
                menu_token: menu.then_some(view.menu_token),
            });
        }
    }
    pub fn hide_palette(&mut self) {
        if let Self::Live { view, connection } = self {
            if let Some(r) = &view.script_result {
                connection.cancel_result(r.token);
            }
        }
        match self {
            Self::Fixture(s) => {
                s.visible = false;
                s.actions_open = false;
            }
            Self::Live { view, connection } => {
                view.visible = false;
                view.actions_open = false;
                view.editor = None;
                connection.action(Action::Key("cancelOperation:".into()));
            }
        }
    }
    pub fn cancel_editor(&mut self) {
        if let Self::Live { view, connection } = self {
            if let Some(editor) = view.editor.take() {
                connection.action(Action::CancelEditor {
                    token: editor.token,
                });
            }
        }
    }
    pub fn cancel_input_control(&self, attempt: u64) {
        if let Self::Live { connection, .. } = self {
            connection.cancel_input_control(attempt);
        }
    }
    pub fn input_control(
        &self,
        value: String,
        operation: ruycast_native::connected::InputOperation,
    ) -> Option<(u64, u64)> {
        if let Self::Live { view, connection } = self {
            let e = view.editor.as_ref().filter(|e| e.run_inline)?;
            Some((e.token, connection.input_control(e.token, value, operation)))
        } else {
            None
        }
    }
    pub fn preview_input(&self, value: String) {
        if let Self::Live { view, connection } = self {
            if let Some(editor) = view.editor.as_ref().filter(|e| {
                e.mode == ruycast_native::actions::Mode::Input
                    && e.task_date.is_some()
                    && !view.execution_running
            }) {
                connection.action(Action::PreviewInput { token: editor.token, value });
            }
        }
    }
    pub fn input_menu(&self, name: Option<String>, key: Option<String>) {
        if let Self::Live { view, connection } = self {
            if let Some(e) = view.editor.as_ref().filter(|e| e.run_inline) {
                connection.action(Action::InputMenu {
                    token: e.token,
                    menu_token: view.menu_token,
                    name,
                    key,
                });
            }
        }
    }
    pub fn set_query_composing(&mut self, query: &str, marked: bool) {
        match self {
            Self::Fixture(s) => s.set_query(query),
            Self::Live { view, connection } => {
                if let Some(r) = &view.script_result {
                    if !r.busy && !r.detail_visible {
                        connection.query_result(r.token, query, marked);
                    }
                } else {
                    connection.query_composing(query, marked)
                }
            }
        }
    }
    pub fn template_token(&self) -> Option<u64> {
        match self {
            Self::Live { view, .. } if view.visible => view
                .editor
                .as_ref()
                .filter(|e| e.template_fields.is_some())
                .map(|e| e.token),
            _ => None,
        }
    }
    pub fn edit_token(&self) -> Option<u64> {
        match self {
            Self::Live { view, .. } if view.visible => view
                .editor
                .as_ref()
                .filter(|e| e.mode == ruycast_native::actions::Mode::Edit)
                .map(|e| e.token),
            _ => None,
        }
    }
    pub fn submit_template(&self, values: Vec<(String, String)>) -> Option<(u64, u64)> {
        if let Self::Live { view, connection } = self {
            let editor = view
                .editor
                .as_ref()
                .filter(|e| e.template_fields.is_some())?;
            Some((
                editor.token,
                connection.submit_template(editor.token, values),
            ))
        } else {
            None
        }
    }
    pub fn submit_input(&self, value: String) -> Option<(u64, u64)> {
        if let Self::Live { view, connection } = self {
            let editor = view
                .editor
                .as_ref()
                .filter(|e| e.mode == ruycast_native::actions::Mode::Input)?;
            Some((editor.token, connection.submit_input(editor.token, value)))
        } else {
            None
        }
    }
    pub fn save_editor(
        &self,
        aliases: Option<String>,
        hotkey: Option<Option<String>>,
    ) -> Option<(u64, u64)> {
        if self.editor_saving() {
            return None;
        }
        if let Self::Live { view, connection } = self {
            let token = view.editor.as_ref()?.token;
            Some((token, connection.save_editor(token, aliases, hotkey, None)))
        } else {
            None
        }
    }
    pub fn save_command(&self, command: serde_json::Value) -> Option<(u64, u64)> {
        if self.editor_saving() {
            return None;
        }
        if let Self::Live { view, connection } = self {
            let token = view.editor.as_ref()?.token;
            Some((
                token,
                connection.save_editor(token, None, None, Some(command)),
            ))
        } else {
            None
        }
    }
    pub fn theme(&self) -> Option<&str> {
        match self {
            Self::Fixture(_) => None,
            Self::Live { view, .. } => view.theme.as_deref(),
        }
    }
    pub fn message(&self) -> &str {
        match self {
            Self::Fixture(_) => "",
            Self::Live { view, .. } if view.script_result.is_some() => &view.message,
            Self::Live { view, .. } => view
                .editor
                .as_ref()
                .map(|e| e.error.as_str())
                .filter(|e| !e.is_empty())
                .unwrap_or(&view.message),
        }
    }
    pub fn query(&self) -> &str {
        match self {
            Self::Fixture(s) => &s.query,
            Self::Live { view, .. } => &view.query,
        }
    }
    pub fn selection(&self) -> Option<usize> {
        match self {
            Self::Fixture(s) => s.selection,
            Self::Live { view, .. } => view.selection,
        }
    }
    pub fn visible(&self) -> bool {
        match self {
            Self::Fixture(s) => s.visible,
            Self::Live { view, .. } => view.visible,
        }
    }
    pub fn activated_id(&self) -> Option<&str> {
        match self {
            Self::Fixture(s) => s.activated_id.as_deref(),
            Self::Live { .. } => None,
        }
    }
    pub fn set_visible(&mut self, value: bool) {
        match self {
            Self::Fixture(s) => s.visible = value,
            Self::Live { view, .. } => view.visible = value,
        }
    }
    pub fn dismissal(&self) -> u64 {
        match self {
            Self::Fixture(_) => 0,
            Self::Live { view, .. } => view.dismissal,
        }
    }
    pub fn results_revision(&self) -> u64 {
        match self {
            Self::Fixture(s) => s.results_revision(),
            Self::Live { view, .. } => view.rows_revision,
        }
    }
    pub fn row_count(&self) -> usize {
        match self {
            Self::Fixture(s) => s.row_count(),
            Self::Live { view, .. } => view.rows.len(),
        }
    }
    pub fn row(&self, index: usize) -> Option<RowRef<'_>> {
        match self {
            Self::Fixture(s) => s.row(index).map(|c| RowRef {
                id: &c.id,
                icon: presentation::icon(c),
                detail: None,
                keyword:c.keyword.as_deref(),
                title: &c.title,
                kind: &c.kind,
                subtitle: None,
                system: false,
                app_shortcut: None,
                hotkey: c.hotkey.as_deref(),
                action: presentation::action_title(c),
                is_group: false,
                aliases: &c.aliases,
                enabled: c.enabled != Some(false),
            }),
            Self::Live { view, .. } => view.rows.get(index).map(|c| RowRef {
                id: &c.id,
                icon: c.icon.clone(),
                detail: c.detail.as_deref(),
                keyword:c.keyword.as_deref(),
                title: &c.title,
                kind: &c.kind,
                subtitle: view.system_subtitles.get(&c.id).map(String::as_str).or_else(||view.script_result.as_ref().map(|_| c.kind.as_str())),
                system: view.system_subtitles.contains_key(&c.id),
                app_shortcut: match view.system_metadata.get(&c.id){Some(ruycast_native::system_search::Metadata::Menu{menu})=>menu.shortcut.as_ref(),_=>None},
                hotkey: c.hotkey.as_deref(),
                action: c.action,
                is_group: c.is_group,
                aliases: &c.aliases,
                enabled: c.enabled,
            }),
        }
    }
    pub fn selected(&self) -> Option<RowRef<'_>> {
        self.selection().and_then(|i| self.row(i))
    }
    pub fn set_query(&mut self, query: &str) {
        match self {
            Self::Fixture(s) => s.set_query(query),
            Self::Live { view, connection } => {
                if let Some(r) = &view.script_result {
                    if !r.busy && !r.detail_visible {
                        connection.query_result(r.token, query, false);
                    }
                } else {
                    connection.query(query)
                }
            }
        }
    }
    pub fn key(&mut self, key: &str, marked: bool) -> bool {
        if let Some(r) = self.script_result() {
            if marked {
                return false;
            }
            if key == "cancelOperation:" {
                self.result_back();
                return true;
            }
            if ["moveDown:", "moveUp:", "insertTab:", "insertNewline:"].contains(&key) {
                if r.busy || r.detail_visible || !r.query_valid {
                    return true;
                }
                let name = match key {
                    "moveUp:" => "up",
                    "insertNewline:" => "choose",
                    _ => "down",
                };
                let id = (name == "choose")
                    .then(|| {
                        r.selection
                            .and_then(|i| r.rows.get(i))
                            .map(|row| row.id.clone())
                    })
                    .flatten();
                if name != "choose" || id.is_some() {
                    self.result_action(r.token, name, id);
                }
                return true;
            }
            return false;
        }
        match self {
            Self::Fixture(s) => s.key(key, marked),
            Self::Live { view, connection } => {
                if !marked
                    && view.actions_open
                    && view.editor.as_ref().is_some_and(|e| e.run_inline)
                {
                    if key == "cancelOperation:" {
                        view.actions_open = false;
                        connection.action(Action::CloseActions {
                            token: view.menu_token,
                        });
                    } else {
                        connection.action(Action::InputMenu {
                            token: view.editor.as_ref().unwrap().token,
                            menu_token: view.menu_token,
                            key: Some(key.into()),
                            name: None,
                        });
                    }
                    return true;
                }
                if !marked && key == "cancelOperation:" {
                    if let Some(editor) = view.editor.take() {
                        connection.action(Action::CancelEditor {
                            token: editor.token,
                        });
                        return true;
                    }
                    if view.actions_open {
                        view.actions_open = false;
                        connection.action(Action::CloseActions {
                            token: view.menu_token,
                        });
                        return true;
                    }
                }
                if !marked && view.actions_open {
                    connection.action(Action::MenuKey {
                        key: key.into(),
                        target: view.action_target.clone(),
                        menu_token: view.menu_token,
                    });
                    return true;
                }
                if marked
                    || ![
                        "moveDown:",
                        "moveUp:",
                        "insertTab:",
                        "insertNewline:",
                        "cancelOperation:",
                    ]
                    .contains(&key)
                {
                    false
                } else {
                    if key == "cancelOperation:" {
                        view.visible = false;
                    }
                    connection.action(Action::Key(key.into()));
                    true
                }
            }
        }
    }
    pub fn select_row(&mut self, index: Option<usize>) {
        if let Some(r) = self.script_result() {
            if !r.busy && r.query_valid {
                if let Some(row) = index.and_then(|i| r.rows.get(i)) {
                    self.result_action(r.token, "select", Some(row.id.clone()));
                }
            }
            return;
        }
        match self {
            Self::Fixture(s) => s.select_row(index),
            Self::Live { view, connection } => connection.action(Action::Select {
                id: index
                    .and_then(|i| view.rows.get(i))
                    .filter(|r| !r.is_group)
                    .map(|c| c.id.clone()),
                catalog_revision: view.catalog_revision,
                epoch: view.epoch,
            }),
        }
    }
    pub fn activate_row(&mut self, index: Option<usize>) {
        if let Some(r) = self.script_result() {
            if !r.busy && r.query_valid && !r.detail_visible {
                if let Some(row) = index.and_then(|i| r.rows.get(i)) {
                    self.result_action(r.token, "activate", Some(row.id.clone()));
                }
            }
            return;
        }
        match self {
            Self::Fixture(s) => {
                s.key("insertNewline:", false);
            }
            Self::Live { view, connection } => {
                if let Some(row) = index.and_then(|i| view.rows.get(i)) {
                    if row.is_group {
                        connection.action(Action::Library {
                            kind: "group".into(),
                            value: row.id.clone(),
                            catalog_revision: view.catalog_revision,
                            previous: None,
                        });
                        return;
                    }
                    connection.action(Action::Activate {
                        id: row.id.clone(),
                        catalog_revision: view.catalog_revision,
                        epoch: view.epoch,
                    });
                }
            }
        }
    }
    pub fn move_selection(&mut self, delta: isize) {
        if let Self::Fixture(s) = self {
            s.move_selection(delta);
        }
    }
    pub fn replace_commands(&mut self, commands: Vec<Command>) {
        if let Self::Fixture(s) = self {
            s.replace_commands(commands);
        }
    }
}
