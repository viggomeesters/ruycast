//! Serialized Rust state owner. Current headless entry implements the migrated
//! model/control contracts; unported native effects fail explicitly.
use crate::connected::{Action, Connection, View};
use crate::{
    control::{ControlServer, HostEvent, Limits},
    observe::ConfigurationObserver,
    profile::ProfilePaths,
    state::HostState,
};
use serde_json::{json, Value};
#[path="runtime_system.rs"]
mod system;
#[path="runtime_utilities.rs"]
mod utilities;
#[path = "runtime_private.rs"]
mod private;
use std::{
    sync::{mpsc, Arc},
    time::{Duration, Instant},
};

#[derive(Clone)]
struct SnippetHideOrigin {
    ui: std::sync::Weak<Connection>,
    epoch: u64,
    dismissal: u64,
    intent: u64,
    native_serial: Option<u64>,
    slot: crate::paste::HideSlot,
}
impl SnippetHideOrigin {
    fn live(&self) -> bool {
        self.ui.upgrade().is_some_and(|ui| {
            !ui.stop_requested()
                && ui.latest_epoch() == self.epoch
                && ui.latest_dismissal() == self.dismissal
                && ui.latest_intent() == self.intent
                && self
                    .slot
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .as_ref()
                    .is_none_or(|token| ui.open_hide_live(&token.key))
        })
    }
    fn capture(ui: &Arc<Connection>, native_serial: Option<u64>) -> Self {
        Self {
            ui: Arc::downgrade(ui),
            epoch: ui.latest_epoch(),
            dismissal: ui.latest_dismissal(),
            intent: ui.latest_intent(),
            native_serial,
            slot: Default::default(),
        }
    }
}
fn log_secrets(values: &Value) -> Vec<String> {
    values.as_object().into_iter().flat_map(|v| v.values()).filter_map(Value::as_str).map(str::to_owned).collect()
}
struct ExecutionContext {
    capture_result: Option<Value>,
    initial_result: Option<u64>,
    analytics: Option<crate::analytics::Span>,
    snippet_hide: Option<SnippetHideOrigin>,
    destination: ExecutionDestination,
    command: rustcast_core::Command,
    file: std::path::PathBuf,
    trace: String,
    logged_id: String,
    log_secrets: Vec<String>,
    started: Instant,
    origin: ExecutionOrigin,
    requested_key: Option<String>,
    _services: crate::main_services::Permit,
}
#[derive(Clone)]
enum ExecutionOrigin {
    Clipboard(u64),
    Saved,
    Discovered(u64),
    Utility,
    Service(u64),
}
enum ExecutionDestination {
    Cli(crate::control::Envelope),
    Native {
        source: &'static str,
        ui: std::sync::Weak<Connection>,
        epoch: u64,
        dismissal: u64,
        intent: u64,
        serial: u64,
        input_token: Option<u64>,
        result_origin: Option<Arc<crate::script_session::Origin>>,
    },
}
impl ExecutionDestination {
    fn source(&self) -> &'static str {
        match self {
            Self::Cli(_) => "cli",
            Self::Native { source, .. } => source,
        }
    }
}
impl ExecutionContext {
    fn source(&self) -> &'static str {
        self.destination.source()
    }
}

struct SharedState {
    permissions:crate::permissions::Source,
    clipboard: Option<crate::clipboard_history::Store>,
    system: system::Shared,
    utility:utilities::State,
    keyboard: Option<crate::keyboard_service::Client>,
    deferred_hotkey: Option<crate::hotkeys::RoutedPress>,
    writer: Option<crate::configuration_jobs::Writer<ConfigPayload>>,
    config_context: Option<ConfigContext>,
    config_dirty: bool,
    events: mpsc::SyncSender<HostEvent>,
    fixture: bool,
    services: Option<crate::main_services::Client>,
    open_validation_stamp: Option<(u64, u64, bool, std::path::PathBuf)>,
    configuration: crate::profile::ConfigStore,
    ranking: crate::ranking::RankingStore,
    execution: crate::execution::Manager<ExecutionContext>,
    activity: crate::activity::Activity,
    diagnostics: crate::analytics_dashboard::Exporter,
    analytics: crate::analytics::Writer,
    applications: Vec<rustcast_core::Command>,
    app_choices: crate::app_choice::Choices,
    applications_revision: u64,
    discovery_pending: bool,
    discovery_error: Option<String>,
}
enum ConfigAfter {
    None,
    Navigate { kind: String, value: String },
    Named { name: String, origin: Option<u64> },
    Select(String),
}
struct ConfigPayload {
    editing: Option<crate::actions::Editing>,
    warning: Option<String>,
    error: Option<String>,
    after: ConfigAfter,
    response: Option<Value>,
}
impl ConfigPayload {
    fn reload(store: &mut crate::profile::ConfigStore) -> Result<Self, String> {
        store.reload()?;
        Ok(Self::response(json!({"ok":true,"result":{
            "commands":store.current().commands.len(),
            "hotkeyWarnings":if store.needs_effects() { store.last_error.iter().collect::<Vec<_>>() } else { vec![] }
        }})))
    }
    fn response(value: Value) -> Self {
        Self {
            editing: None,
            warning: None,
            error: None,
            after: ConfigAfter::None,
            response: Some(value),
        }
    }
}
struct ConfigOrigin {
    draft_live: Option<crate::execution::Live>,
    ui: std::sync::Weak<Connection>,
    epoch: u64,
    intent: u64,
    dismissal: u64,
    deadline: Instant,
    before: Option<crate::actions::EditorView>,
}
impl ConfigOrigin {
    fn ownership(&self) -> bool {
        self.ui.upgrade().is_some_and(|ui| {
            !ui.stop_requested()
                && ui.latest_dismissal() == self.dismissal
                && self.before.as_ref().is_none_or(|e| {
                    !ui.editor_cancelled(e.token)
                        && e.origin_inline.is_none_or(|t| !ui.editor_cancelled(t))
                        && e.origin_menu.is_none_or(|t| !ui.menu_cancelled(t))
                        && e.suspended_input_token
                            .is_none_or(|t| !ui.editor_cancelled(t))
                })
        })
    }
    fn clocks(&self) -> bool {
        self.ownership()
            && self.ui.upgrade().is_some_and(|ui| {
                ui.latest_epoch() == self.epoch && ui.latest_intent() == self.intent
            })
    }
    fn storage_live(&self) -> bool {
        // An admitted autosave belongs to this immutable item/draft, not to
        // the following query. Cancel/hide/parent retirement still revoke it.
        Instant::now() < self.deadline
            && self.ownership()
            && self.draft_live.as_ref().is_none_or(|live| live())
    }
    fn live(&self) -> bool {
        Instant::now() < self.deadline && self.clocks()
    }
}

enum ConfigContext {
    Watch,
    WatchThenCli(crate::control::Envelope),
    Cli(crate::control::Envelope),
    Native(std::sync::Arc<ConfigOrigin>),
    #[cfg(feature = "test-tools")]
    Owned(OwnedRequest),
}
impl SharedState {
    fn healthy(&self,headless:bool,permissions:Option<&crate::permissions::Snapshot>)->bool {
        let hooks=self.keyboard.as_ref().map(|k|k.hook_health()).unwrap_or_default();
        let keyword=self.keyboard.as_ref().map(|k|k.keyword_status()).unwrap_or_else(||"Off".into());
        crate::runtime_health::healthy(crate::runtime_health::Requirements::from_config(self.configuration.current()),&crate::runtime_health::Known{
            permissions,
            storage_error:self.configuration.last_error.is_some()||self.clipboard.as_ref().is_some_and(|s|s.pause_error.is_some()||s.cleanup_error.is_some()||s.warning.is_some())||self.analytics.client().storage_error().is_some()||self.activity.storage_error.is_some(),
            headless,keyboard_present:self.keyboard.is_some(),keyboard_error:self.keyboard.as_ref().is_some_and(|k|k.error().is_some()),keyboard_pending:self.configuration.needs_effects(),recording:self.keyboard.as_ref().is_some_and(|k|k.recording()),paused:self.keyboard.as_ref().is_some_and(|k|k.paused()),hooks:&hooks,keyword:&keyword,
        })
    }
    fn config_busy(&self) -> bool {
        self.writer.as_ref().is_some_and(|w| w.busy())
    }
    fn queue_reload(
        &mut self,
        context: ConfigContext,
        live: crate::execution::Live,
        force: bool,
    ) -> Result<(), String> {
        let writer = self
            .writer
            .as_mut()
            .ok_or("Configuration writer unavailable.")?;
        writer.start(&self.configuration, live, force, ConfigPayload::reload)?;
        self.config_context = Some(context);
        Ok(())
    }
    fn admit_cli_reload(&mut self, request: crate::control::Envelope) {
        if !request.context.is_live() {
            let response=request.context.cancel_response();
            let _=request.respond(response); return;
        }
        if request.value["configPath"].as_str().is_some_and(|p|std::path::Path::new(p)!=self.configuration.file) {
            let _=request.respond(json!({"ok":false,"error":"Configuration folder differs from the running app. Restart Ruycast to use the selected folder."}));return;
        }
        if self.config_busy() {
            if matches!(self.config_context,Some(ConfigContext::Watch)) {
                self.config_context=Some(ConfigContext::WatchThenCli(request));
            } else {
                let _=request.respond(json!({"ok":false,"error":"Configuration is being updated. Try again."}));
            }
            return;
        }
        let check=request.context.clone();
        let publication=request.context.configuration_publication();
        let result=self.writer.as_mut().expect("runtime writer").start(&self.configuration,Arc::new(move||check.is_live()),false,move|store|{store.bind_publication(publication);ConfigPayload::reload(store)});
        if let Err(error)=result {let _=request.respond(json!({"ok":false,"error":error}));} else {self.config_context=Some(ConfigContext::Cli(request));}
    }
    fn cancel_configuration_reply(&mut self, deliveries: &mut Vec<mpsc::Receiver<bool>>) {
        match self.config_context.take() {
            Some(ConfigContext::Cli(request)|ConfigContext::WatchThenCli(request))=>{let response=request.context.cancel_response();deliveries.push(request.respond(response));}
            #[cfg(feature="test-tools")]
            Some(ConfigContext::Owned(request))=>{request.cancelled.store(true,std::sync::atomic::Ordering::Release);let _=request.response.try_send(Err("Catalog staging stopped; configuration publication may be unknown.".into()));}
            _=>{}
        }
    }
    fn configuration_changed(&mut self) {
        // A delayed observer notice may describe bytes already adopted by an
        // explicit reload. Do not start a redundant writer over fresh requests.
        if !self.config_busy() && !self.config_dirty && self.configuration.last_error.is_none() && !self.configuration.needs_effects()
            && self.configuration.snapshot_is_current(Some(32_000_000)) == Ok(true)
        {
            return;
        }
        self.config_dirty = true;
        if !self.config_busy()
            && self.queue_reload(ConfigContext::Watch, Arc::new(|| true), false).is_ok()
        {
            self.config_dirty = false;
        }
    }
    fn reload_watched(&mut self) -> Result<bool, String> {
        if self.config_busy() || self.config_dirty {
            return Err("Configuration is being updated. Try again.".into());
        }
        if self.writer.as_ref().is_some_and(|w| w.has_keyboard()) {
            if self.configuration.snapshot_is_current(Some(32_000_000))?
                && !self.configuration.needs_effects()
            {
                return Ok(false);
            }
            self.queue_reload(
                ConfigContext::Watch,
                Arc::new(|| true),
                self.configuration.needs_effects(),
            )?;
            return Err("Configuration is being updated. Try again.".into());
        }
        self.configuration.reload_limited(Some(32_000_000))
    }
}
type Stamp = (
    u64,
    Option<usize>,
    bool,
    u64,
    u64,
    String,
    Option<String>,
    Option<String>,
    u64,
    (bool, u64, usize, Option<crate::actions::EditorView>),
    (
        u64,
        u64,
        Option<(u64, bool)>,
        bool,
        (u64, bool, u64, Option<u64>),
    ),
    (Option<crate::connected::PassiveNotice>, Option<Arc<crate::analytics_dashboard::Surface>>, Option<(u64,u64,u64)>, bool, Option<String>, Option<String>, Option<crate::settings_surface::ClipboardStatus>, u64, u64, bool),
);
struct FieldRequest {
    source: &'static str,
    generation: u64,
    command: rustcast_core::Command,
    file: std::path::PathBuf,
    config_revision: u64,
    epoch: u64,
    intent: u64,
    dismissal: u64,
    alias: bool,
    launch: bool,
    deadline: Instant,
}
struct CreationRequest {
    token: u64,
    file: std::path::PathBuf,
    revision: u64,
    epoch: u64,
    intent: u64,
    dismissal: u64,
    menu: Option<u64>,
    deadline: Instant,
    _permit: crate::main_services::Permit,
    reply: Option<crate::main_services::Prefill>,
    fixture_at: Option<Instant>,
    requested: bool,
}
struct WindowRuntime {
    clipboard_preview: Option<crate::clipboard_preview::Worker>,
    clipboard_preview_key: Option<(String,u64)>,
    system: system::Owner,
    utility:utilities::Window,
    dashboard: crate::analytics_dashboard::Owner,
    dashboard_request: Option<(u64,u64,u64)>,
    // One admitted save may wait behind an internal watcher, never behind a
    // competing user mutation. Preserve its original token/attempt/deadline.
    deferred_save: Option<crate::connected::Input>,
    native_source: &'static str,
    passive_notice: Option<crate::connected::PassiveNotice>,
    notice_serial: u64,
    creation_request: Option<CreationRequest>,
    field_preparation: crate::template_form::Preparation,
    field_request: Option<FieldRequest>,
    session: crate::session::Session,
    editing: crate::actions::Editing,
    epoch: u64,
    visibility_intent: Option<Instant>,
    query_intent: Option<Instant>,
    message: String,
    #[cfg(feature = "test-tools")]
    last_named_refusal: Option<Value>,
    dismissal: u64,
    show_serial: u64,
    input_intent: u64,
    execution_serial: u64,
    execution_effect: Option<(u64, bool)>,
    config_revision: u64,
    ranking_revision: u64,
    usage_revision: u64,
    applications_revision: u64,
    clipboard_revision: u64,
    clipboard_return_query: Option<String>,
    filtered_clip_ids: Vec<String>,
    clipboard_query: String,
    clipboard_selection: isize,
    clipboard_data_revision: u64,
    clipboard_results_revision: Option<u64>,
    stamp: Option<Stamp>,
    last_view: Option<View>,
    results: crate::script_session::Owner,
    row_by_id: std::collections::HashMap<String, usize>,
}
impl WindowRuntime {
    fn new(
        session: crate::session::Session,
        editing: crate::actions::Editing,
        shared: &SharedState,
    ) -> Self {
        let field_preparation = crate::template_form::Preparation::new(shared.events.clone());
        #[cfg(feature = "test-tools")]
        let field_preparation = if shared.fixture {
            field_preparation.fixture(shared.configuration.paths.home.clone())
        } else {
            field_preparation
        };
        Self {
            clipboard_preview:None,
            clipboard_preview_key:None,
            system: Default::default(),
            utility:Default::default(),
            dashboard: Default::default(),
            dashboard_request: None,
            deferred_save: None,
            native_source: "launcher",
            passive_notice: None,
            notice_serial: 0,
            creation_request: None,
            field_preparation,
            field_request: None,
            session,
            editing,
            epoch: 0,
            visibility_intent: None,
            query_intent: None,
            message: String::new(),
            #[cfg(feature = "test-tools")]
            last_named_refusal: None,
            dismissal: 0,
            show_serial: 0,
            input_intent: 0,
            execution_serial: 0,
            execution_effect: None,
            config_revision: u64::MAX,
            ranking_revision: shared.ranking.revision,
            usage_revision: u64::MAX,
            applications_revision: shared.applications_revision,
            clipboard_revision:u64::MAX,
            clipboard_return_query:None,
            filtered_clip_ids:Vec::new(),
            clipboard_query:String::new(),
            clipboard_selection:0,
            clipboard_data_revision:u64::MAX,
            clipboard_results_revision:None,
            stamp: None,
            last_view: None,
            row_by_id: Default::default(),
            results: Default::default(),
        }
    }
    fn retire_creation(&mut self) {
        if let Some(r) = self.creation_request.take() {
            self.editing.cancel(r.token);
            self.stamp = None;
        }
    }
    #[cfg(any(test,feature = "test-tools"))]
    fn filtered_clips<'a>(&self,state:&'a SharedState)->Vec<&'a rustcast_core::Clip> {
        let Some(store)=&state.clipboard else {return Vec::new()};
        let current=store.items.iter().map(|clip|(clip.id.as_str(),clip)).collect::<std::collections::HashMap<_,_>>();
        self.filtered_clip_ids.iter().filter_map(|id|current.get(id.as_str()).copied()).collect()
    }
    fn capture_clipboard_results(&mut self) {
        if !self.session.clipboard_mode {return;}
        let revision=self.session.results_revision();
        if self.clipboard_results_revision!=Some(revision)||self.clipboard_data_revision!=self.clipboard_revision {
            self.filtered_clip_ids=self.session.rows().filter_map(|row|row.id.strip_prefix("clipboard:").map(str::to_owned)).collect();
            self.clipboard_results_revision=Some(revision);
            self.clipboard_data_revision=self.clipboard_revision;
        }
        if self.clipboard_query!=self.session.query {
            self.clipboard_query.clone_from(&self.session.query);
            self.clipboard_selection=0;
        }
        if let Some(index)=self.session.selection {self.clipboard_selection=index as isize;}
    }
    fn prune_hidden_clipboard_results(&mut self,state:&SharedState) {
        let Some(store)=&state.clipboard else {self.filtered_clip_ids.clear();return;};
        if self.clipboard_data_revision==store.revision {return;}
        let current=store.items.iter().map(|clip|clip.id.as_str()).collect::<std::collections::HashSet<_>>();
        // Match the original hidden publication: keep its order and resolve
        // current payloads by identity; do not search or add new matches here.
        self.filtered_clip_ids.retain(|id|current.contains(id.as_str()));
        self.clipboard_data_revision=store.revision;
    }
    fn sync(&mut self, state: &SharedState) {
        if !self.session.clipboard_mode {self.prune_hidden_clipboard_results(state);}
        state.system.invalidate_if_revision(state.configuration.revision);
        if self.utility.mode.is_some() { return; }
        if self.session.clipboard_mode {
            if let Some(store)=&state.clipboard {if self.clipboard_revision!=store.revision {
                let query_changed=self.clipboard_query!=self.session.query;
                if query_changed {self.clipboard_selection=0;}
                let selected=(self.session.visible&&!query_changed&&self.clipboard_results_revision.is_some()).then(||usize::try_from(self.clipboard_selection).ok()
                    .and_then(|i|self.filtered_clip_ids.get(i)).cloned()).flatten();
                self.session.replace_commands(store.items.iter().map(clipboard_command).collect());
                if let Some(selected)=selected {
                    self.clipboard_selection=self.session.rows().position(|row|row.id.strip_prefix("clipboard:")==Some(selected.as_str())).map_or(-1,|i|i as isize);
                }
                self.session.select_row(usize::try_from(self.clipboard_selection).ok());
                self.clipboard_revision=store.revision;self.stamp=None;
            }}self.capture_clipboard_results();return;
        }
        if self.usage_revision != state.activity.usage_revision {
            if self.session.library.is_none() {
                self.session
                    .catalog
                    .set_usage(state.activity.counts.clone());
                self.session.refresh_preserving_selection();
            }
            self.usage_revision = state.activity.usage_revision;
        }
        if self.config_revision != state.configuration.revision
            || (self.session.library.is_none()
                && self.applications_revision != state.applications_revision)
        {
            let commands = if self.session.library.is_some() {
                crate::management_commands::catalog(&state.configuration.current().commands)
            } else {
                crate::applications::merge(
                    &crate::management_commands::catalog(&state.configuration.current().commands),
                    &state.applications,
                )
            };
            self.session.replace_commands(commands);
            self.config_revision = state.configuration.revision;
            self.applications_revision = state.applications_revision;
        }
        if self.ranking_revision != state.ranking.revision {
            if self.session.library.is_none() {
                self.session
                    .catalog
                    .prepare_recency(state.ranking.entries());
                self.session.refresh_preserving_selection();
            }
            self.ranking_revision = state.ranking.revision;
        }
    }
    fn notify(&mut self, text: String, success: bool) {
        self.notice_serial = self.notice_serial.wrapping_add(1);
        self.passive_notice = Some(crate::connected::PassiveNotice {
            serial: self.notice_serial,
            epoch: self.epoch,
            intent: self.input_intent,
            dismissal: self.dismissal,
            text,
            success,
        });
    }
    fn publish(&mut self, state: &SharedState, ui: &Arc<Connection>) {
        self.sync(state);
        let clipboard=if self.session.clipboard_mode && self.session.visible {
            state.clipboard.as_ref().map(|store|{
                let clip=usize::try_from(self.clipboard_selection).ok().and_then(|i|self.filtered_clip_ids.get(i)).and_then(|id|store.items.iter().find(|c|&c.id==id));
                let key=clip.map(|c|(c.id.clone(),store.revision));
                if self.clipboard_preview.is_none(){self.clipboard_preview=crate::clipboard_preview::Worker::new(state.fixture).ok();}
                if self.clipboard_preview_key!=key {
                    if let Some(preview)=&mut self.clipboard_preview{preview.load(&state.configuration.paths.root,clip);}
                    self.clipboard_preview_key=key;self.stamp=None;
                }
                if let Some(preview)=&mut self.clipboard_preview{if preview.poll(){self.stamp=None;}}
                let preview=self.clipboard_preview.as_ref();
                crate::clipboard_history::Surface{revision:store.revision,selected_id:clip.map(|c|c.id.clone()),text:clip.map(|c|c.text.clone()).unwrap_or_default(),source:clip.map(|c|c.source.clone()).unwrap_or_default(),date:clip.map(|c|c.date.clone()).unwrap_or_default(),kind:clip.and_then(|c|c.content_type.clone()).unwrap_or_else(||"text".into()),files:clip.and_then(|c|c.file_urls.clone()).unwrap_or_default(),paused:store.paused,loading:preview.is_some_and(|p|p.loading),preview:preview.and_then(|p|p.image.clone()),error:preview.and_then(|p|p.error.clone())}
            })
        }else{if let Some(mut preview)=self.clipboard_preview.take(){preview.load(&state.configuration.paths.root,None);}self.clipboard_preview_key=None;None};
        let clipboard_status=state.clipboard.as_ref().map(|s|crate::settings_surface::ClipboardStatus{count:s.items.len(),paused:s.paused,pause_error:s.pause_error.clone(),cleanup_error:s.cleanup_error.clone(),warning:s.warning.clone()});
        system::tick(self,state,ui);
        let dashboard_active = self.session.visible && self.session.library.as_ref().is_some_and(|n|n.tab=="dashboard");
        self.dashboard.activate(dashboard_active);
        if dashboard_active {
            if let Some(token)=self.results.token(){ui.cancel_result(token);self.results.clear();}
        }
        let dashboard = if dashboard_active {
            Some(self.dashboard.surface(&state.analytics.client(), ||crate::management_commands::catalog(&state.configuration.current().commands).into_iter().chain(state.applications.iter().cloned()).collect(), crate::activity::now(), state.diagnostics.busy(), [state.configuration.revision,state.applications_revision]))
        } else { None };
        let executing = state.execution.any(|c| matches!(&c.destination,ExecutionDestination::Native{ui:owner,..} if std::ptr::eq(owner.as_ptr(),ui.as_ref())));
        let stamp = (
            self.session.results_revision(),
            self.session.selection,
            self.session.visible,
            state.configuration.revision,
            self.epoch,
            self.message.clone(),
            state.configuration.last_error.clone(),
            state.configuration.current().theme.clone(),
            self.dismissal,
            (
                self.session.actions_open,
                self.editing.menu_token,
                self.editing.action_selection,
                self.editing.view(),
            ),
            (
                self.show_serial,
                self.input_intent,
                self.execution_effect,
                executing,
                self.results.stamp(),
            ),
            (self.passive_notice.clone(), dashboard.clone(), self.dashboard_request, state.config_busy(), state.analytics.client().storage_error(), state.keyboard.as_ref().and_then(|k|k.error()),clipboard_status.clone(),self.session.navigation_revision, state.system.monitor.revision, state.healthy(false,None)),
        );
        if self.stamp.as_ref() == Some(&stamp) {
            return;
        }
        self.stamp = Some(stamp);
        let message = if self.message.is_empty() {
            state
                .configuration
                .last_error
                .as_ref()
                .map(|error| format!("Config: {error}"))
                .unwrap_or_else(|| {
                    if executing {
                        "Running…".into()
                    } else {
                        String::new()
                    }
                })
        } else {
            self.message.clone()
        };
        let cached = self.last_view.as_ref().filter(|v| {
            v.catalog_revision == self.session.results_revision() && v.script_result.is_none()
        });
        let mut view = if let Some(cached) = cached {
            let mut view = cached.clone();
            view.selection = self
                .session
                .selected()
                .and_then(|c| self.row_by_id.get(&c.id).copied());
            view
        } else {
            let view = View::from_session(
                &self.session,
                self.epoch,
                state.configuration.revision,
                state.configuration.current().theme.clone(),
                message.clone(),
            );
            self.row_by_id = view
                .rows
                .iter()
                .enumerate()
                .filter(|(_, r)| !r.is_group)
                .map(|(i, r)| (r.id.clone(), i))
                .collect();
            view
        };
        if self.session.clipboard_mode {
            if let Some(store) = &state.clipboard {
                for row in Arc::make_mut(&mut view.rows) {
                    if let Some(clip) = row.id.strip_prefix("clipboard:").and_then(|id| store.items.iter().find(|c| c.id == id)) {
                        row.icon = crate::presentation::Icon::Symbol(match clip.content_type.as_deref() { Some("image")=>"photo", Some("files")=>"doc", _=>"doc.text" });
                        row.detail = Some(clip.date.clone());
                    }
                }
            }
        }
        let config=state.configuration.current();
        let requires_input=crate::runtime_health::Requirements::from_config(config).input();
        view.menu_bar=Some(crate::menu_bar::State{requires_input,visible:state.configuration.current().menu_bar_visible.unwrap_or(true),healthy:state.healthy(false,None)});
        view.settings = if self.session.visible && self.editing.view().is_none() {
            self.session.library.as_ref().filter(|n|["general","advanced","clipboard"].contains(&n.tab.as_str())).map(|n|Arc::new(crate::settings_surface::Surface {
                navigation_revision:self.session.navigation_revision,
                tab:n.tab.clone(),revision:state.configuration.revision,epoch:self.epoch,dismissal:self.dismissal,
                configuration:serde_json::from_slice(state.configuration.snapshot()).unwrap_or_default(),
                directory:state.configuration.file.parent().map(|p|p.display().to_string()).unwrap_or_default(),
                path:state.configuration.file.display().to_string(),busy:state.config_busy(),message:message.clone(),clipboard:clipboard_status.clone(),
            }))
        } else {None};
        view.clipboard=clipboard;
        if view.clipboard.is_some(){view.action_items.clear();}
        view.utility_scope=self.utility.mode.clone();
        view.utility_busy=self.utility.busy;
        if let Some(mode)=&self.utility.mode {
            let action=if utilities::kind(mode)==Some("text"){ "Use Text" }else{"Copy"};
            view.rows=Arc::new(view.rows.iter().cloned().map(|mut row|{row.action=action;row}).collect());
        }
        view.dashboard = dashboard;
        view.monitor=if ui.is_management{None}else{state.system.monitor.surface()};
        view.dashboard_request = self.dashboard_request;
        view.execution_running = executing || self.system.action_running();
        view.system_scope = Some(self.system.scope);
        view.process_sort=self.system.process_sort;
        view.system_busy = self.system.busy();
        view.system_metadata = self.system.snapshot.metadata.clone();
        view.system_subtitles = system::subtitles(&self.system.snapshot);
        view.completion_intent = self.execution_effect.map(|e| e.0);
        view.visible = self.session.visible;
        view.epoch = self.epoch;
        view.config_revision = state.configuration.revision;
        view.theme = state.configuration.current().theme.clone();
        view.message = if view.clipboard.is_some() || view.settings.as_ref().is_some_and(|s|s.tab=="clipboard") {
            clipboard_status.as_ref().and_then(|s|s.error()).map(str::to_owned).unwrap_or(message)
        } else { message };
        if self.utility.busy && view.message.is_empty(){view.message="Loading…".into();}
        view.passive_notice = self.passive_notice.clone();
        view.dismissal = self.dismissal;
        view.show_serial = self.show_serial;
        view.selection_scroll_serial = self.session.selection_scroll_serial;
        view.actions_open = self.session.actions_open;
        view.menu_token = self.editing.menu_token;
        view.action_selection = self.editing.action_selection;
        view.action_target = crate::actions::target(&self.session, self.epoch);
        view.action_items = crate::actions::items(
            self.session.selected(),
            self.session.selected().is_some_and(|c| {
                state
                    .configuration
                    .current()
                    .commands
                    .iter()
                    .any(|saved| saved.id == c.id)
            }),
        );
        system::extend_items(&mut view.action_items,self);
        if self.editing.run_inline() {
            view.action_items = input_items(&self.editing);
            view.action_target = None;
        }
        view.editor = if self.creation_request.is_some() {
            None
        } else {
            self.editing.view()
        };
        view.app_choices = state.app_choices.clone();
        view.app_choices_pending = state.discovery_pending;
        view.separate_command_keys = state
            .configuration
            .current()
            .separate_command_keys
            .unwrap_or(true);
        if let Some(result) = self.results.surface() {
            view.query = result.query.clone().unwrap_or_default();
            view.rows = Arc::new(if result.detail_visible {
                vec![]
            } else {
                result
                    .rows
                    .iter()
                    .map(|r| crate::connected::Row {
                        id: r.id.clone(),
                        icon: crate::presentation::Icon::Symbol("command"),
                        detail: None,
                        keyword: None,
                        title: r.title.clone(),
                        kind: r.subtitle.clone(),
                        hotkey: None,
                        imported_hotkey: None,
                        action: "Choose",
                        is_group: false,
                        aliases: vec![],
                        enabled: result.query_valid && !result.busy,
                    })
                    .collect()
            });
            view.selection = if result.detail_visible {
                None
            } else {
                result.selection
            };
            view.actions_open = false;
            view.action_items.clear();
            view.action_target = None;
            view.message = if result.error.is_empty() && result.busy {
                "Running…".into()
            } else {
                result.error.clone()
            };
            view.script_result = Some(result);
        }
        if view.clipboard.is_some() {
            view.rows=Arc::new(view.rows.iter().cloned().map(|mut r|{r.action="Paste";r.kind="Clipboard".into();r}).collect());
            view.action_items = if self.session.selected().is_some(){[("clipboard.paste","Paste"),("clipboard.copy","Copy"),("clipboard.delete","Delete")].into_iter().map(|(id,title)|crate::actions::Item{id,title:title.into(),shortcut:None}).collect()}else{vec![]};
            view.action_items.push(crate::actions::Item{id:"clipboard.settings",title:"Settings".into(),shortcut:None});
            view.action_target=crate::actions::target(&self.session,self.epoch);
        }
        self.last_view = Some(view.clone());
        ui.publish(view);
    }
    fn show(
        &mut self,
        query: Option<&str>,
        received: Instant,
        ui: &Connection,
    ) -> Result<(), String> {
        if self.visibility_intent.is_some_and(|last| received < last) {
            return Err("A newer palette action superseded this request.".into());
        }
        ui.begin_open_trace(self.show_serial.wrapping_add(1));
        if self.session.clipboard_mode {self.capture_clipboard_results();self.session.clipboard_mode=false;self.config_revision=u64::MAX;}
        self.session.launcher_scope = None;
        self.native_source = "launcher";
        if !ui.is_management { self.system.enter(crate::system_search::Scope::MainTabs); }
        self.utility=Default::default();
        self.field_preparation.cancel();
        self.field_request = None;
        ui.cancel_open_hide();
        self.visibility_intent = Some(received);
        if self.query_intent.is_none_or(|last| received >= last) {
            if let Some(query) = query {
                self.session.set_query(query);
                self.query_intent = Some(received);
                ui.adopt_query(query, self.epoch);
            }
        }
        if let Some(t) = self.results.token() {
            ui.cancel_result(t);
        }
        self.results.clear();
        self.execution_effect = None;
        self.session.visible = true;
        self.show_serial = self.show_serial.wrapping_add(1);
        self.session.actions_open = false;
        self.message.clear();
        // Opening existing Settings preserves its open draft; palette show starts a fresh search.
        if !ui.is_management {
            self.editing.cancel_all();
        }
        Ok(())
    }
    fn status(&self, ui: &Connection) -> Value {
        let (revision, display_rows) = ui.view_summary();
        json!({"utilityMode":self.utility.mode,"utilityBusy":self.utility.busy,"systemSearchBusy":self.system.busy(),"systemActionRunning":self.system.action_running(),"systemScope":format!("{:?}",self.system.scope),"creationPreparing":self.creation_request.is_some(),"templatePreparing":self.field_request.is_some(),"connected":true,"revision":revision,"appliedRevision":ui.applied_revision(),"visible":self.session.visible,"queryEpoch":self.epoch,"selectedId":self.session.selected().map(|c|&c.id),"rows":self.session.row_count(),"displayRows":display_rows,"editorOpen":self.editing.token().is_some(),"tab":self.session.library.as_ref().map(|n|&n.tab),"filter":self.session.library.as_ref().map(|n|&n.filter)})
    }
}

pub fn run(paths: ProfilePaths) -> Result<Value, String> {
    let (tx, rx) = mpsc::sync_channel(32);
    run_connected(paths, tx, rx, None)
}
#[cfg(feature = "test-tools")]
pub fn run_fixture(paths: ProfilePaths) -> Result<Value, String> {
    let (tx, rx) = mpsc::sync_channel(32);
    run_with_services(paths, tx, rx, None, true, None)
}
pub fn run_connected(
    paths: ProfilePaths,
    tx: mpsc::SyncSender<HostEvent>,
    rx: mpsc::Receiver<HostEvent>,
    ui: Option<Arc<Connection>>,
) -> Result<Value, String> {
    let fixture = ui.as_ref().is_some_and(|ui| ui.hidden_tests);
    run_with_services(paths, tx, rx, ui, fixture, None)
}
pub fn run_with_services(
    paths: ProfilePaths,
    tx: mpsc::SyncSender<HostEvent>,
    rx: mpsc::Receiver<HostEvent>,
    ui: Option<Arc<Connection>>,
    fixture: bool,
    services: Option<crate::main_services::Client>,
) -> Result<Value, String> {
    run_graph(paths, tx, rx, ui, fixture, services, None)
}

/// The owner channel is minted only by consuming the affine constructor
/// admission. Snapshot is readonly; the closed measured operations use the
/// actual production owner, including AnalyticsTiming's real mutation.
/// Ordinary control requests cannot construct this channel's owner.
#[cfg(feature = "test-tools")]
pub struct OwnedRuntime {
    host: crate::owned_host_entry::QualifiedOwnedHost,
    requests: mpsc::Receiver<OwnedRequest>,
}
#[cfg(feature = "test-tools")]
pub struct OwnedClient {
    requests: mpsc::SyncSender<OwnedRequest>,
    events: mpsc::SyncSender<HostEvent>,
}
#[cfg(feature = "test-tools")]
struct OwnedRequest {
    deadline: Instant,
    operation: OwnedOperation,
    cancelled: Arc<std::sync::atomic::AtomicBool>,
    response: mpsc::SyncSender<Result<Value, String>>,
}
#[cfg(feature = "test-tools")]
#[derive(Clone,Copy)]
pub enum OwnedOperation {
    Snapshot,
    StageCatalog(usize),
    CachedResultRead,
    StageProcessRecency(u64),
    StageProcesses1000,
    ValidateProcesses1000,
    CachedProcesses1000,
    ProcessSort1000,
    ConfigRawRead,
    AnalyticsTiming,
}
// Private qualified-host preparation uses the original PerformanceChecks data.
// It publishes into the existing Session/system owner, never into a second model.
#[cfg(feature = "test-tools")]
fn owned_process_fixture() -> Result<crate::system_search::Results,String> {
    let commands=crate::native_f1::fixture::process_commands().into_iter()
        .map(serde_json::from_value).collect::<Result<Vec<_>,_>>().map_err(|e|e.to_string())?;
    let metadata=(0..1000).map(|i| (format!("fixture.process.{i}"),crate::system_search::Metadata::Process {
        process:crate::processes::Resource {
            identity:crate::system_search::Identity {pid:200000+i,uid:unsafe{libc::getuid()},seconds:i as u64,micros:0},
            name:format!("Process {i}"),path:"/usr/bin/true".into(),cpu_percent:(i%97) as f64,memory_bytes:i as u64*1024,
        }
    })).collect();
    Ok(crate::system_search::Results {commands,metadata,foreground:None})
}
#[cfg(feature = "test-tools")]
fn owned_process_valid(window:&WindowRuntime, fixture:&crate::system_search::Results)->Result<(),String> {
    if window.system.scope!=crate::system_search::Scope::Processes
        || window.system.snapshot.metadata!=fixture.metadata
        || serde_json::to_value(&window.system.snapshot.commands).ok()!=serde_json::to_value(&fixture.commands).ok()
        || window.session.row_count()!=1000
        || !window.session.query.is_empty()
        || window.session.selected().is_none_or(|c|c.id!="fixture.process.96")
        || !matches!(window.system.process_sort,crate::processes::Sort::Cpu|crate::processes::Sort::Memory)
    {return Err("The original 1000-process owner fixture is not current.".into());}
    let expected=fixture.commands.iter().map(|c|(c.id.as_str(),c)).collect::<std::collections::HashMap<_,_>>();
    let mut ids=std::collections::HashSet::new();
    for row in window.session.rows() {
        if !ids.insert(row.id.as_str()) || expected.get(row.id.as_str()).is_none_or(|c|serde_json::to_value(c).ok()!=serde_json::to_value(row).ok()) {
            return Err("The 1000-process commands changed before measurement.".into());
        }
    }
    // Closed original fixture order, independent of the implementation's
    // sorting routine. Validation must never repair or reorder measured state.
    let order:Vec<usize>=match window.system.process_sort {
        crate::processes::Sort::Cpu=>(0..97).rev().flat_map(|cpu|(0..1000).filter(move|i|i%97==cpu)).collect(),
        crate::processes::Sort::Memory=>(0..1000).rev().collect(),
        _=>return Err("The process fixture sort is outside its measured lanes.".into()),
    };
    if window.session.rows().map(|row|row.id.as_str()).ne(order.iter().map(|i|format!("fixture.process.{i}"))) {
        return Err("The process fixture row order changed before measurement.".into());
    }
    Ok(())
}
#[cfg(feature = "test-tools")]
fn owned_process_operation(window:&mut WindowRuntime,state:&SharedState,ui:Option<&Arc<Connection>>,request:&OwnedRequest)->Result<Option<Value>,String> {
    if !request.live() {return Err("Owned process operation expired or stopped.".into());}
    if ui.is_none_or(|ui|ui.hidden_tests||ui.is_management||ui.stop_requested())
        || window.session.visible || window.session.clipboard_mode || window.session.launcher_scope.is_some()
        || window.session.library.is_some() || window.session.actions_open || window.editing.token().is_some()
        || window.results.token().is_some() || window.utility.mode.is_some() || window.system.busy()
        || state.config_busy() || state.config_dirty
    {return Err("The process fixture requires the idle qualified hidden palette owner.".into());}
    window.sync(state);
    let fixture=owned_process_fixture()?;
    if matches!(request.operation,OwnedOperation::StageProcesses1000) {
        if window.system.scope==crate::system_search::Scope::Processes {
            owned_process_valid(window,&fixture)?;
        } else {
            if !request.live(){return Err("Owned process preparation expired before publication.".into());}
            window.system.enter(crate::system_search::Scope::Processes);
            // AppState.mode's original transition clears its previous query.
            // Keep that real owner mutation outside both process timers.
            window.session.set_query("");
            state.system.remember(&fixture);
            window.system.snapshot=fixture;
            window.session.set_external_results(window.system.snapshot.commands.clone(),false);
            system::process_sort(window,crate::processes::Sort::Cpu);
            // applySystemResults(preserveSelection:false) selects the first
            // ordered row, rather than preserving the original unsorted row 0.
            window.session.select_row(Some(0));
        }
        if !request.live(){return Err("Owned process publication may have completed after expiry.".into());}
        return Ok(None);
    }
    owned_process_valid(window,&fixture)?;
    if !request.live(){return Err("Owned process operation expired before its timer.".into());}
    if matches!(request.operation,OwnedOperation::ValidateProcesses1000) {
        return Ok(None);
    }
    let start=Instant::now();
    match request.operation {
        OwnedOperation::CachedProcesses1000=>{},
        OwnedOperation::ProcessSort1000=>{
            let next=if window.system.process_sort==crate::processes::Sort::Cpu {crate::processes::Sort::Memory}else{crate::processes::Sort::Cpu};
            system::process_sort(window,next);
        },
        _=>return Err("This operation is not a process measurement.".into()),
    }
    let visible_count=matches!(request.operation,OwnedOperation::CachedProcesses1000).then(||window.session.row_count());
    let selected_id_count=window.session.selected().map_or(0,|c|c.id.chars().count());
    let elapsed=start.elapsed().as_nanos();
    if !request.live(){return Err("Owned process operation expired after its measured work.".into());}
    // Validate the actual post-operation owner before its full snapshot is
    // serialized below; this fixture rule does not change generic sorting.
    owned_process_valid(window,&fixture)?;
    Ok(Some(json!({"elapsed_ns":u64::try_from(elapsed).map_err(|_|"Owned timer overflow.")?,"result":{"visible_count":visible_count,"selected_id_count":selected_id_count}})))
}
#[cfg(feature = "test-tools")]
fn owned_catalog_activation(store: &crate::profile::ConfigStore)->Result<(),String> {
    if store.needs_effects() {
        return Err("Catalog activation is pending; no catalog measurement.".into());
    }
    if store.last_error.is_some() {
        return Err("Catalog configuration has an unresolved warning; no catalog measurement.".into());
    }
    Ok(())
}
#[cfg(feature = "test-tools")]
fn owned_process_recency(state:&mut SharedState,request:&OwnedRequest,epoch:u64)->Result<(),String> {
    // The endpoint supplies the independently pinned fixture epoch. Preserve
    // exact integer timestamps when passing through the real f64 store API.
    if !(511..=9_007_199_254_740_991).contains(&epoch)||!request.live() {
        return Err("Invalid or expired owned ranking preparation.".into());
    }
    let durable=|receipt:crate::profile::SaveReceipt|->Result<(),String>{
        if !receipt.durable||receipt.warning.is_some(){Err("Owned ranking publication is unresolved.".into())}else{Ok(())}
    };
    durable(state.ranking.reset()?)?;
    for i in 0..512 {
        if !request.live(){return Err("Owned ranking preparation expired; publication may be partial.".into());}
        durable(state.ranking.record_success(&format!("fixture.{i}"),(epoch-(511-i)) as f64)?)?;
    }
    let expected=serde_json::from_value::<std::collections::HashMap<String,f64>>(
        crate::native_f1::fixture::recency(epoch as i64)["entries"].clone()).map_err(|e|e.to_string())?;
    let persisted=crate::ranking::RankingStore::open(&state.configuration.paths.root,epoch as f64);
    if state.ranking.entries()!=&expected||persisted.entries()!=state.ranking.entries()||!request.live(){
        return Err("Owned ranking fixture changed or expired after publication.".into());
    }
    Ok(())
}
#[cfg(feature = "test-tools")]
fn owned_catalog_adoption(store:&crate::profile::ConfigStore,uncertain:bool,payload:ConfigPayload)->Result<Value,String> {
    if uncertain {return Err("Configuration publication is uncertain; no catalog measurement.".into());}
    owned_catalog_activation(store)?;
    Ok(payload.response.unwrap_or_else(||json!({"ok":false})))
}
#[cfg(feature = "test-tools")]
struct OwnedWait(Arc<std::sync::atomic::AtomicBool>);
#[cfg(feature = "test-tools")]
impl Drop for OwnedWait {
    fn drop(&mut self) { self.0.store(true,std::sync::atomic::Ordering::Release); }
}
#[cfg(feature = "test-tools")]
impl OwnedRequest {
    fn live(&self)->bool {
        !self.cancelled.load(std::sync::atomic::Ordering::Acquire)&&Instant::now()<self.deadline
    }
}
#[cfg(feature = "test-tools")]
pub fn owned_channel(
    host: crate::owned_host_entry::QualifiedOwnedHost,
    events: mpsc::SyncSender<HostEvent>,
) -> (OwnedRuntime, OwnedClient) {
    let (sender, requests) = mpsc::sync_channel(1);
    (OwnedRuntime { host, requests }, OwnedClient { requests: sender, events })
}
#[cfg(feature = "test-tools")]
impl OwnedClient {
    pub fn inspect(&self, deadline: Instant) -> Result<Value, String> {
        self.perform(OwnedOperation::Snapshot,deadline)
    }
    pub fn perform(&self, operation: OwnedOperation, deadline: Instant) -> Result<Value, String> {
        let remaining = deadline.checked_duration_since(Instant::now())
            .ok_or("Owned runtime inspection expired.")?;
        let (response, reply) = mpsc::sync_channel(1);
        let cancelled=Arc::new(std::sync::atomic::AtomicBool::new(false));
        let _wait=OwnedWait(cancelled.clone());
        self.requests.try_send(OwnedRequest { deadline, operation, cancelled, response })
            .map_err(|_| "Owned runtime inspection is busy or stopped.")?;
        let _ = self.events.try_send(HostEvent::UiWake);
        reply.recv_timeout(remaining).map_err(|_| match operation {
            OwnedOperation::StageCatalog(_)|OwnedOperation::StageProcessRecency(_)|OwnedOperation::StageProcesses1000=>"Owned fixture request expired or stopped; publication may be unknown.".to_string(),
            _=>"Owned runtime request expired or stopped.".to_string(),
        })?
    }
}
#[cfg(feature = "test-tools")]
impl OwnedRuntime {
    pub fn run(
        self,
        tx: mpsc::SyncSender<HostEvent>,
        rx: mpsc::Receiver<HostEvent>,
        ui: Arc<Connection>,
        services: crate::main_services::Client,
    ) -> Result<Value, String> {
        if ui.hidden_tests||ui.is_management {
            return Err("Owned production runtime requires the ordinary primary connection.".into());
        }
        run_graph(self.host.into_paths(), tx, rx, Some(ui), false, Some(services), Some(self.requests))
    }
}
#[cfg(not(feature = "test-tools"))]
type OwnedRequest = ();

fn run_graph(
    paths: ProfilePaths,
    tx: mpsc::SyncSender<HostEvent>,
    rx: mpsc::Receiver<HostEvent>,
    ui: Option<Arc<Connection>>,
    fixture: bool,
    services: Option<crate::main_services::Client>,
    owned_requests: Option<mpsc::Receiver<OwnedRequest>>,
) -> Result<Value, String> {
    #[cfg(not(feature = "test-tools"))]
    let _ = owned_requests;
    let roots = if fixture {
        #[cfg(feature = "test-tools")]
        {
            crate::applications::fixture_roots(&paths)?
        }
        #[cfg(not(feature = "test-tools"))]
        {
            return Err("App discovery fixture mode is unavailable.".into());
        }
    } else {
        crate::applications::system_roots(&paths.home)
    };
    #[allow(unused_mut)]
    let mut transport_limits = Limits::default();
    #[cfg(feature = "test-tools")]
    if fixture {
        if let Some(value) = std::env::var_os("RUYCAST_NATIVE_TEST_DISPATCH_MS") {
            let ms = value
                .to_str()
                .and_then(|s| s.parse::<u64>().ok())
                .filter(|n| (100..=3000).contains(n))
                .ok_or("Invalid own dispatch fixture limit")?;
            transport_limits.dispatch = Duration::from_millis(ms);
        }
    }
    let mut server = ControlServer::start(&paths.root, tx.clone(), transport_limits)?;
    let HostState {
        configuration,
        ranking,
        session,
        editing,
    } = HostState::open(paths.clone())?;
    let keyboard = services.as_ref().and_then(|s| s.keyboard());
    let initial_keyboard = keyboard.is_some();
    let mut config_writer = crate::configuration_jobs::Writer::new(
        server.profile_lease(),
        tx.clone(),
        keyboard.clone(),
    );
    if let Some(services) = &services { config_writer.bind_capture(&services.clipboard_capture); }
    let clipboard=Some(crate::clipboard_history::Store::open_with_retention(&paths.root,&configuration.current().clipboard,config_writer.retention_phase())?);
    let mut system = system::Shared::new(fixture);
    system.main_tabs_enabled = ui.is_some();
    #[cfg(feature = "test-tools")]
    if fixture && std::env::var("RUYCAST_NATIVE_TEST_MAIN_TABS").as_deref() == Ok("1") {
        system.main_tabs_enabled = true;
    }
    let mut state = SharedState {
        permissions:crate::permissions::Source::new(fixture),
        clipboard,
        system,
        utility:utilities::State::new(&paths,fixture,Some(server.profile_lease()),tx.clone())?,
        keyboard,
        deferred_hotkey: None,
        writer: Some(config_writer),
        config_context: None,
        config_dirty: false,
        events: tx.clone(),
        fixture,
        services,
        open_validation_stamp: None,
        configuration,
        ranking,
        execution: crate::execution::Manager::new(paths.home.clone(), tx.clone()),
        activity: crate::activity::Activity::open(&paths.root)?,
        diagnostics: Default::default(),
        analytics: crate::analytics::Writer::open(&paths.root, Some(server.profile_lease()))?,
        applications: vec![],
        app_choices: Default::default(),
        applications_revision: 0,
        discovery_pending: true,
        discovery_error: None,
    };
    if let (Some(store),Some(services))=(&state.clipboard,&state.services){services.clipboard_capture.publish(&state.configuration.current().clipboard,store.paused);}
    let analytics_events = tx.clone();
    state.analytics.client().attach_wake(Arc::new(move || {
        let _ = analytics_events.try_send(HostEvent::UiWake);
    }));
    state
        .analytics
        .client()
        .import_legacy(&state.activity.counts);
    if let Some(keyboard) = &state.keyboard {
        keyboard.attach_analytics(state.analytics.client());
    }
    let mut discovery = crate::applications::Discovery::start(roots, tx.clone())?;
    let mut palette = WindowRuntime::new(session, editing, &state);
    let management_ui = ui.as_ref().and_then(|ui| ui.peer());
    let mut management = WindowRuntime::new(
        if management_ui.is_some() {
            crate::session::Session::management(state.configuration.current().commands.clone())
        } else {
            crate::session::Session::new(vec![])
        },
        Default::default(),
        &state,
    );
    let mut observer = ConfigurationObserver::start(paths, tx)?;
    if initial_keyboard {
        state.queue_reload(ConfigContext::Watch, Arc::new(|| true), true)?;
    }
    let mut clipboard_settings_revision=state.configuration.revision;
    let mut last_clipboard_maintenance=Instant::now();
    let mut quit = false;
    let mut shutdown_deliveries: Vec<mpsc::Receiver<bool>> = vec![];
    let mut analytics_responses: Vec<(crate::control::Envelope, mpsc::Receiver<()>, bool)> = vec![];
    if let Some(ui) = &ui {
        palette.publish(&state, ui);
    }
    if let Some(ui) = &management_ui {
        management.publish(&state, ui);
    }
    while server.is_running() && !quit {
        #[cfg(feature = "test-tools")]
        if let Some(requests) = &owned_requests {
            if let Ok(request) = requests.try_recv() {
                if let OwnedOperation::StageCatalog(size)=request.operation {
                    // Keep the normal HostEvent receive below reachable even
                    // under repeated expired/busy requests on this channel.
                    (||{
                    if !request.live() {
                        let _=request.response.try_send(Err("The catalog request expired.".into()));
                        return;
                    }
                    if state.config_busy()||state.config_dirty {
                        // A definite pre-admission observation. The client may
                        // wait for this existing writer, never retry uncertainty.
                        let _=request.response.try_send(Ok(json!({"busy":true})));
                        return;
                    }
                    if let Err(error)=owned_catalog_activation(&state.configuration) {
                        let _=request.response.try_send(Err(error));
                        return;
                    }
                    let configuration=(||->Result<Value,String>{
                        let value=serde_json::from_slice::<Value>(state.configuration.snapshot()).map_err(|e|e.to_string())?;
                        crate::native_f1::fixture::configuration(&value,size)
                    })();
                    let value=match configuration {Ok(value)=>value,Err(error)=>{let _=request.response.try_send(Err(error));return;}};
                    let unchanged=serde_json::from_slice::<Value>(state.configuration.snapshot()).is_ok_and(|old|old==value);
                    let cancelled=request.cancelled.clone(); let deadline=request.deadline;
                    let result=state.writer.as_mut().expect("The production runtime owns its configuration writer.").start(
                        &state.configuration,
                        Arc::new(move||!cancelled.load(std::sync::atomic::Ordering::Acquire)&&Instant::now()<deadline),
                        false,
                        move|store|{
                            let mut payload=if unchanged {ConfigPayload::reload(store)?}else{
                                let receipt=store.save(&value)?;
                                ConfigPayload::response(json!({"ok":receipt.durable,"durabilityWarning":receipt.warning}))
                            };
                            let response=payload.response.get_or_insert_with(||json!({"ok":false}));
                            response["ok"]=json!(response["ok"]==true&&owned_catalog_activation(store).is_ok());
                            response["keyboardActivationPending"]=json!(store.needs_effects());
                            response["configurationWarning"]=json!(store.last_error);
                            // Preserve the real publication outcome and warning:
                            // a pending activation is not a failed/rolled-back write.
                            Ok(payload)
                        });
                    match result {
                        Ok(())=>state.config_context=Some(ConfigContext::Owned(request)),
                        Err(error)=>{let _=request.response.try_send(Err(error));}
                    }
                    })();
                } else {
                let observed = if !request.live() {
                    Err("Owned runtime inspection expired.".into())
                } else { (||->Result<Value,String>{
                    let measured = match request.operation {
                        OwnedOperation::Snapshot => None,
                        OwnedOperation::StageProcessRecency(epoch)=>{
                            if ui.as_ref().is_none_or(|ui|ui.hidden_tests||ui.is_management||ui.stop_requested())
                                || palette.session.visible||palette.session.clipboard_mode||palette.session.launcher_scope.is_some()
                                || palette.session.library.is_some()||palette.session.actions_open||palette.editing.token().is_some()
                                || palette.results.token().is_some()||palette.utility.mode.is_some()||palette.system.busy()
                                || state.config_busy()||state.config_dirty
                                || palette.session.query!="m"
                                || serde_json::to_value(&state.configuration.current().commands).ok()!=Some(json!(crate::native_f1::fixture::catalog(10000)?)) {
                                return Err("Owned ranking preparation requires the hidden original 10k catalog/query owner.".into());
                            }
                            owned_process_recency(&mut state,&request,epoch)?;
                            palette.sync(&state);
                            None
                        }
                        OwnedOperation::StageProcesses1000|OwnedOperation::ValidateProcesses1000|OwnedOperation::CachedProcesses1000|OwnedOperation::ProcessSort1000=>owned_process_operation(&mut palette,&state,ui.as_ref(),&request)?,
                        OwnedOperation::StageCatalog(_)=>unreachable!("Catalog mutations use the existing configuration writer."),
                        OwnedOperation::CachedResultRead => {
                            let start=Instant::now();
                            let count=palette.session.row_count();
                            let elapsed=start.elapsed().as_nanos();
                            Some(json!({"elapsed_ns":u64::try_from(elapsed).map_err(|_|"Owned timer overflow.")?,"result":count}))
                        }
                        OwnedOperation::ConfigRawRead => {
                            let io=state.configuration.paths.io()?.ok_or("The owned configuration has no pinned IO provider.")?;
                            let start=Instant::now();
                            let bytes=io.read_optional(&state.configuration.file,32_000_000)?.ok_or("The configuration disappeared before its measured read.")?;
                            let count=bytes.len();
                            let elapsed=start.elapsed().as_nanos();
                            Some(json!({"elapsed_ns":u64::try_from(elapsed).map_err(|_|"Owned timer overflow.")?,"result":{"bytes":count}}))
                        }
                        OwnedOperation::AnalyticsTiming => {
                            let start=Instant::now();
                            state.analytics.client().timing("search.model",0.1);
                            let elapsed=start.elapsed().as_nanos();
                            Some(json!({"elapsed_ns":u64::try_from(elapsed).map_err(|_|"Owned timer overflow.")?,"result":state.analytics.client().dashboard_snapshot().0}))
                        }
                    };
                    // These are observations of the actual serialized owners,
                    // not a second Session or a reread of configuration files.
                    serde_json::from_slice::<Value>(state.configuration.snapshot()).map(|configuration|json!({
                        "configuration":configuration,
                        "measured":measured,
                        "configuration_revision":state.configuration.revision,
                        "ranking_revision":state.ranking.revision,
                        "keyboardActivationPending":state.configuration.needs_effects(),
                        "configurationWarning":state.configuration.last_error,
                        "recency":state.ranking.entries(), "usage":state.activity.counts,
                        "palette_epoch":palette.epoch,
                        "palette":{
                            "rows":palette.session.rows().collect::<Vec<_>>(),
                            "selection":if palette.session.clipboard_mode {json!(palette.clipboard_selection)}else{json!(palette.session.selection)},
                            "actionsOpen":palette.session.actions_open,
                            "config":configuration,
                            "filteredClips":palette.filtered_clips(&state),
                            "query":palette.session.query,
                            "mode":if palette.session.clipboard_mode {"clipboard"}else if palette.system.scope==crate::system_search::Scope::Processes {"processes"}else{"commands"},
                            "processSort":match palette.system.process_sort {crate::processes::Sort::Cpu=>"cpu",crate::processes::Sort::Memory=>"memory",crate::processes::Sort::Name=>"name"},
                            "processResources":palette.system.snapshot.metadata.iter().filter_map(|(id,m)|if let crate::system_search::Metadata::Process{process}=m {Some((id,process))}else{None}).collect::<std::collections::BTreeMap<_,_>>(),
                            "message":palette.message,
                            "busy":state.execution.count()>0,
                            "clipboardSearchBusy":false,
                            "paletteVisible":palette.session.visible
                        },
                        "owners":{
                            "executions":state.execution.count(),
                            "execution_workers":state.execution.live_workers(),
                            "snippet_transactions":crate::paste::pending_tickets(),
                            "configuration_writer":usize::from(state.config_busy()),
                            "configuration_reply":usize::from(state.config_context.is_some()),
                            "configuration_dirty":usize::from(state.config_dirty),
                            "analytics_replies":analytics_responses.len(),
                            "shutdown_replies":shutdown_deliveries.len(),
                            "diagnostics_export":usize::from(state.diagnostics.busy()),
                            "discovery":usize::from(state.discovery_pending),
                            "system":usize::from(state.system.busy()),
                            "palette_system":usize::from(palette.system.busy()),
                            "management_system":usize::from(management.system.busy()),
                            "palette_clipboard_preview":usize::from(palette.clipboard_preview.as_ref().is_some_and(|p|p.loading)),
                            "management_clipboard_preview":usize::from(management.clipboard_preview.as_ref().is_some_and(|p|p.loading)),
                            "analytics_versions":state.analytics.client().dashboard_version(),
                            "palette_template":palette.field_preparation.owned_pending(),
                            "management_template":management.field_preparation.owned_pending(),
                            "utility_jobs":state.utility.owned_pending(),
                            "palette_utility":palette.utility.busy,
                            "management_utility":management.utility.busy,
                            "palette_deferred_save":usize::from(palette.deferred_save.is_some()),
                            "management_deferred_save":usize::from(management.deferred_save.is_some()),
                            "palette_creation":usize::from(palette.creation_request.is_some()),
                            "management_creation":usize::from(management.creation_request.is_some()),
                            "palette_field":usize::from(palette.field_request.is_some()),
                            "management_field":usize::from(management.field_request.is_some()),
                            "palette_input":ui.as_ref().map(|u|u.pending_owned_inputs()),
                            "management_input":management_ui.as_ref().map(|u|u.pending_owned_inputs())
                        },
                        "owner_coverage_complete":false,
                        "unobserved_owners":["host_event_queue","main_services","sdk_effects","retired_system_workers"]
                    })).map_err(|_|"The runtime owner has no valid configuration snapshot.".into())
                })()
                };
                let _ = request.response.try_send(observed);
                }
            }
        }
        shutdown_deliveries.retain(|ack| matches!(ack.try_recv(), Err(mpsc::TryRecvError::Empty)));
        let mut waiting = vec![];
        for (request, done, reset) in analytics_responses.drain(..) {
            match done.try_recv() {
                Ok(()) => {
                    let _ = request.respond(analytics_response(&state.analytics.client(), reset));
                }
                Err(mpsc::TryRecvError::Empty) if request.context.is_live() => {
                    waiting.push((request, done, reset))
                }
                Err(_) => {
                    let _ = request.respond(analytics_unconfirmed());
                }
            }
        }
        analytics_responses = waiting;
        if let Some(ui)=&management_ui {
            state.diagnostics.cancel_unless(management.dashboard.token(),ui.latest_dismissal(),!ui.stop_requested()&&management.session.visible&&management.session.library.as_ref().is_some_and(|n|n.tab=="dashboard"));
        }
        if let Some((token, dismissal, message)) = state.diagnostics.poll() {
            if management_ui.as_ref().is_some_and(|ui|ui.latest_dismissal()==dismissal) {
                management.dashboard.export_complete(token,message);
            }
        }
        let wake = state
            .execution
            .next_wake()
            .into_iter()
            .chain(state.utility.next_wake())
            .chain(management.dashboard.wake())
            .chain(Some(last_clipboard_maintenance+Duration::from_secs(60)))
            .chain(palette.clipboard_preview.as_ref().is_some_and(|p|p.loading).then(||Instant::now()+Duration::from_millis(20)))
            .chain(state.diagnostics.busy().then(||Instant::now()+Duration::from_millis(20)))
            .chain(
                palette
                    .creation_request
                    .as_ref()
                    .map(|r| r.deadline.min(Instant::now() + Duration::from_millis(20))),
            )
            .chain(
                management
                    .creation_request
                    .as_ref()
                    .map(|r| r.deadline.min(Instant::now() + Duration::from_millis(20))),
            )
            .chain(palette.system.deadline())
            .chain(management.system.deadline())
            .chain(state.system.busy().then(||Instant::now()+Duration::from_millis(20)))
            .chain(state.system.monitor.deadline())
            .chain(palette.field_request.as_ref().map(|r| r.deadline))
            .chain(management.field_request.as_ref().map(|r| r.deadline))
            .chain(
                state
                    .config_busy()
                    .then(|| Instant::now() + Duration::from_millis(20)),
            )
            .min();
        let event = if let Some(wake) = wake {
            match rx.recv_timeout(wake.saturating_duration_since(Instant::now())) {
                Ok(event) => event,
                Err(mpsc::RecvTimeoutError::Timeout) => HostEvent::UiWake,
                Err(error) => return Err(error.to_string()),
            }
        } else {
            rx.recv().map_err(|e| e.to_string())?
        };
        utilities::poll(&mut state,&mut palette,ui.as_ref());
        finish_configuration(&mut state, &mut palette, &mut management);
        if clipboard_settings_revision!=state.configuration.revision || last_clipboard_maintenance.elapsed()>=Duration::from_secs(60) {
            clipboard_settings_revision=state.configuration.revision;
            last_clipboard_maintenance=Instant::now();let settings=&state.configuration.current().clipboard;
            if let Some(store)=&mut state.clipboard {if let Err(error)=store.update(settings,||true,|base|rustcast_core::history(base,None,settings,&crate::clipboard_history::now())){store.cleanup_error=Some(error);}}
        }

        system::poll_cli(&mut state);
        if !state.config_busy() && !state.config_dirty {
            if let Some(event) = state.deferred_hotkey.take() {
                if let Some(ui) = &ui {
                    handle_hotkey(&mut state, &mut palette, ui, event);
                }
            }
        }
        if let Some(result) = discovery.take_with_choices() {
            state.discovery_pending = false;
            palette.stamp = None;
            management.stamp = None;
            match result {
                Ok((applications, choices)) => {
                    state.app_choices = choices;
                    state.applications = applications;
                    state.applications_revision = state
                        .applications_revision
                        .checked_add(1)
                        .expect("application revision exhausted");
                }
                Err(error) => state.discovery_error = Some(error),
            }
        }
        if let Some(ui) = &ui {
            expire_fields(&mut palette, ui);
        }
        if let Some(ui) = &management_ui {
            expire_fields(&mut management, ui);
        }
        let mut completed = state.execution.harvest();
        let stamp_changed =
            state
                .open_validation_stamp
                .as_ref()
                .is_none_or(|(config, apps, error, file)| {
                    *config != state.configuration.revision
                        || *apps != state.applications_revision
                        || *error != state.configuration.last_error.is_some()
                        || *file != state.configuration.file
                });
        if stamp_changed && state.execution.find_open(|_| true).is_some() {
            // Poll deadlines/liveness without repeatedly rereading config.
            // Watch/reload updates the snapshot; OpenCommit forces actual I/O.
            let current = state.configuration.last_error.is_none();
            state.open_validation_stamp = Some((
                state.configuration.revision,
                state.applications_revision,
                !current,
                state.configuration.file.clone(),
            ));
            let config = &state.configuration;
            let apps = &state.applications;
            let generation = state.applications_revision;
            let clipboard=state.clipboard.as_ref();
            completed.extend(state.execution.retire_open_where(|c| {
                !current || if let ExecutionOrigin::Clipboard(revision)=c.origin { !clipboard.is_some_and(|s|s.revision==revision&&s.items.iter().any(|item|format!("clipboard:{}",item.id)==c.command.id)) }else{!origin_valid_parts(config, apps, generation, c)}
            }));
        }
        palette.sync(&state);
        if management_ui.is_some() {
            management.sync(&state);
        }
        if ui
            .as_ref()
            .is_some_and(|ui| process_inputs(&mut state, &mut palette, ui))
            || management_ui
                .as_ref()
                .is_some_and(|ui| process_inputs(&mut state, &mut management, ui))
        {
            palette.retire_creation();
            management.retire_creation();
            shutdown_deliveries.extend(finish_executions(
                &mut state,
                &mut palette,
                &mut management,
                completed,
            ));
            break;
        }
        if let Some(ui) = &ui {
            finish_creation(&mut state, &mut palette, ui);
            finish_fields(&mut state, &mut palette, ui);
        }
        if let Some(ui) = &management_ui {
            finish_creation(&mut state, &mut management, ui);
            finish_fields(&mut state, &mut management, ui);
        }
        shutdown_deliveries.extend(finish_executions(
            &mut state,
            &mut palette,
            &mut management,
            completed,
        ));
        palette.sync(&state);
        if management_ui.is_some() {
            management.sync(&state);
        }
        if let Some(store)=&state.clipboard {if let Some(services)=&state.services {services.clipboard_capture.publish(&state.configuration.current().clipboard,store.paused);}}
        match event {
            HostEvent::ClipboardCaptureError(error)=>{palette.message=format!("Clipboard could not be saved: {error}");palette.stamp=None;}
            HostEvent::ClipboardCapture(capture)=>{
                if let Some(store)=&mut state.clipboard {
                    if let Err(error)=store.capture(capture.payload,capture.source,capture.at,&state.configuration.current().clipboard,||true){palette.message=format!("Clipboard could not be saved: {error}");}
                }
            }

            HostEvent::SnippetFailure {
                epoch,
                routing,
                dismissal,
                failure,
            } => {
                if let Some(ui) = &ui {
                    if !ui.stop_requested()
                        && ui.latest_epoch() == epoch
                        && ui.routing_revision() == routing
                        && ui.latest_dismissal() == dismissal
                    {
                        palette.notify(failure.text().into(), false);
                    }
                }
            }
            HostEvent::OpenCommit(commit) => {
                handle_open_grant(&mut state, &mut palette, &mut management, commit, false)
            }
            HostEvent::OpenHide(commit) => {
                handle_open_grant(&mut state, &mut palette, &mut management, commit, true)
            }
            HostEvent::PasteRead(commit) => handle_paste_grant(&mut state, &mut palette, commit, 0),
            HostEvent::PasteCommit(commit) => {
                handle_paste_grant(&mut state, &mut palette, commit, 1)
            }
            HostEvent::PasteHide(commit) => handle_paste_grant(&mut state, &mut palette, commit, 2),
            HostEvent::ServerStopped => break,
            HostEvent::UiWake => {}
            HostEvent::Hotkey(event) => {
                if let Some(ui) = &ui {
                    handle_hotkey(&mut state, &mut palette, ui, event);
                }
            }
            HostEvent::ConfigurationChanged => {
                observer.acknowledge();
                state.configuration_changed();
            }
            HostEvent::Request(request) => {
                if !request.context.is_live() {
                    let _ = request
                        .respond(json!({"ok":false,"error":"Request expired or was cancelled."}));
                    continue;
                }
                let mut request=match utilities::request(&mut state,request){Ok(())=>continue,Err(request)=>request};
                if request.value["op"]=="monitor" {system::cli_monitor(&mut state,request);continue;}
                if request.value["op"]=="show" && request.value["mode"]=="monitor" {
                    let result=if ui.is_some(){state.system.monitor.open()}else{Err("The monitor window is unavailable in headless mode. Use ruycast monitor for data.".into())};
                    if result.is_ok(){palette.session.visible=false;}
                    let _=request.respond(result.map(|_|json!({"ok":true,"result":{"state":"opened"}})).unwrap_or_else(|e|json!({"ok":false,"error":e})));continue;
                }
                if request.value["op"] == "reload" {
                    state.admit_cli_reload(request);
                    continue;
                }
                if request.value["op"] == "config.directory" {
                    let Some(directory) = request.value["directory"].as_str().map(str::to_owned)
                    else {
                        let _ = request
                            .respond(json!({"ok":false,"error":"Missing configuration folder."}));
                        continue;
                    };
                    let check = request.context.clone();
                    let publication = request.context.configuration_publication();
                    let result=state.writer.as_mut().expect("runtime writer").start(&state.configuration,Arc::new(move||check.is_live()),false,move |store|{store.bind_publication(publication); let receipt=store.relocate(&directory)?;Ok(ConfigPayload::response(json!({"ok":true,"result":{"directory":store.file.parent(),"path":store.file,"originalRetained":true},"durabilityWarning":receipt.warning})))});
                    if let Err(error) = result {
                        let _ = request.respond(json!({"ok":false,"error":error}));
                    } else {
                        state.config_context = Some(ConfigContext::Cli(request));
                    }
                    continue;
                }
                #[cfg(feature = "test-tools")]
                if state.fixture && request.value["op"] == "workspace.fixture.delay" {
                    if !crate::workspace::fixture_installed(&state.configuration.paths.home) {
                        let _ = request
                            .respond(json!({"ok":false,"error":"Fixture boundary missing."}));
                        continue;
                    }
                    let _ = std::fs::write(
                        state
                            .configuration
                            .paths
                            .home
                            .join("owner-open-delay-started"),
                        b"entered",
                    );
                    std::thread::sleep(Duration::from_millis(500));
                    let _ = request.respond(json!({"ok":true,"result":{"ownDelayCompleted":true}}));
                    continue;
                }
                if request.value["op"] == "native.test"
                    && cfg!(feature = "test-tools")
                    && ui.as_ref().is_some_and(|ui| ui.hidden_tests)
                {
                    #[cfg(feature = "test-tools")]
                    if request.value["action"] == "date-reference" {
                        let result = (|| {
                            let owner = ui.as_ref().ok_or("Hidden native owner required.")?;
                            if !state.fixture || request.value["surface"] == "settings"
                                || !palette.session.visible || palette.editing.cancelled(owner)
                                || palette.editing.is_saving() || state.config_busy() || state.config_dirty || state.config_context.is_some()
                                || owner.stop_requested() || !request.context.is_live()
                                || state.execution.any(|_| true)
                            { return Err("An idle owned date-input fixture is required.".to_owned()); }
                            let token = request.value["token"].as_u64().ok_or("Date input token required.")?;
                            let now = request.value["now"].as_f64().filter(|n|n.is_finite()).ok_or("Finite date reference required.")?;
                            let (command, value, _) = palette.editing.input(token).ok_or("Date input is no longer open.")?;
                            if !crate::task_date::supports(command) { return Err("Date input required.".to_owned()); }
                            let command = command.clone();
                            let value = value.to_owned();
                            let inline = palette.editing.run_inline();
                            palette.editing.open_at(&state.configuration, &command, crate::actions::Mode::Input, now)?;
                            let token = palette.editing.token().expect("date input opened");
                            palette.editing.set_run_inline(token, inline);
                            palette.editing.update_input_at(token, 0, &value, now)?;
                            palette.stamp = None;
                            palette.publish(&state, owner);
                            let date = palette.editing.view().and_then(|e|e.task_date).expect("date preview");
                            Ok(json!({"token":token,"input":date.input,"value":date.value,"label":date.label}))
                        })();
                        let _ = request.respond(match result { Ok(value)=>json!({"ok":true,"result":value}),Err(error)=>json!({"ok":false,"error":error}) });
                        continue;
                    }
                    if let Some(ui) = &ui {
                        palette.publish(&state, ui);
                    }
                    if let Some(ui) = &management_ui {
                        management.publish(&state, ui);
                    }
                    let selected = if request.value["surface"] == "settings" {
                        management_ui.as_ref()
                    } else {
                        ui.as_ref()
                    };
                    if let Some(selected) = selected {
                        selected.forward_probe(request);
                    } else {
                        let _ = request
                            .respond(json!({"ok":false,"error":"Settings window is unavailable."}));
                    }
                    continue;
                }
                if request.value["op"] == "native.delay"
                    && cfg!(feature = "test-tools")
                    && ui.as_ref().is_some_and(|ui| ui.hidden_tests)
                {
                    let start = Instant::now();
                    let before = ui.as_ref().map(|ui| ui.latest_intent());
                    let before_stop = ui.as_ref().is_some_and(|ui| ui.stop_requested());
                    #[cfg(feature = "test-tools")]
                    if request.value["executionBarrier"] == true {
                        // Independent client threads do not imply request order.
                        // Fixtures must observe actual owner entry before they
                        // release a script or enqueue quit on another socket.
                        if let Err(error) = std::fs::write(
                            state.configuration.paths.root.join("owner-delay-started"),
                            b"started",
                        ) {
                            let _ = request.respond(json!({"ok":false,"error":format!("Delay barrier failed: {error}")}));
                            continue;
                        }
                    }
                    #[cfg(feature = "test-tools")]
                    if request.value["holdOwner"] == true
                        && request.value["executionBarrier"] == true
                    {
                        let limit = Instant::now() + Duration::from_secs(5);
                        while !state
                            .configuration
                            .paths
                            .root
                            .join("owner-delay-release")
                            .exists()
                            && Instant::now() < limit
                        {
                            std::thread::sleep(Duration::from_millis(5));
                        }
                    } else {
                        std::thread::sleep(Duration::from_millis(500));
                    }
                    #[cfg(not(feature = "test-tools"))]
                    std::thread::sleep(Duration::from_millis(500));
                    #[cfg(feature = "test-tools")]
                    if request.value["executionBarrier"] == true {
                        let _ = std::fs::write(
                            state.configuration.paths.root.join("owner-delay-finished"),
                            b"finished",
                        );
                    }
                    let closed = ui
                        .as_ref()
                        .is_some_and(|ui| ui.hidden_during(start, Instant::now()));
                    let _ = request
                        .respond(json!({"ok":true,"result":{"localCloseDuringDelay":closed,"executionReady":state.execution.ready_count(),"newIntentDuringDelay":before!=ui.as_ref().map(|ui|ui.latest_intent()),"nativeStopDuringDelay":!before_stop && ui.as_ref().is_some_and(|ui|ui.stop_requested())}}));
                    continue;
                }
                if request.value["op"].as_str().is_some_and(|op|op.starts_with("clipboard.")) {
                    clipboard_request(&mut state,&mut palette,ui.as_ref(),request);
                    continue;
                }
                #[cfg(feature="test-tools")]
                if state.fixture && request.value["op"]=="system.fixture" {let response=system::fixture(&state,&request.value);let _=request.respond(response);continue;}
                if request.value["op"]=="processes" {request.value["op"]=json!("search");request.value["mode"]=json!("processes");request.value["processDetails"]=json!(true);system::cli_search(&mut state,request);continue;}
                #[cfg(feature="test-tools")]
                if state.fixture&&request.value["op"]=="permissions.fixture" {
                    let reply=match(request.value["accessibility"].as_bool(),request.value["inputMonitoring"].as_bool(),request.value["raycastRunning"].as_bool()){
                        (Some(accessibility),Some(input_monitoring),Some(raycast_running))=>json!({"ok":state.permissions.set_fixture(crate::permissions::Snapshot{accessibility,input_monitoring,raycast_running})}),
                        _=>json!({"ok":false,"error":"Owned permission values must be booleans."}),
                    };let _=request.respond(reply);continue;
                }
                if request.value["op"] == "search" && (request.value["mode"].as_str().or(request.value["scope"].as_str()).is_none_or(|s|crate::system_search::Scope::parse(s).is_some())) {
                    system::cli_search(&mut state,request);
                    continue;
                }
                let request = if request.value["op"] == "execute" {match system::cli_open_builtin(&mut state,&mut palette,ui.as_ref(),request){Ok(())=>continue,Err(r)=>r}}else{request};
                let request = if request.value["op"] == "execute" { match system::cli_execute(&mut state,request) {Ok(())=>continue,Err(r)=>r} }else{request};
                if request.value["op"] == "execute" && request.value["dryRun"] != true {
                    admit_request(&mut state, &mut palette, ui.as_ref(), request);
                    continue;
                }
                let mut value = if let (Some("show"), Some(main_ui)) =
                    (request.value["op"].as_str(), ui.as_ref())
                {
                    if request.value["mode"] == "clipboard" {
                        match show_clipboard(&state,&mut palette,main_ui,request.context.received_at()){Ok(())=>json!({"ok":true,"result":{"state":"opened"}}),Err(error)=>json!({"ok":false,"error":error})}
                    } else if request.value["mode"].as_str().and_then(crate::session::LauncherScope::parse).is_some() {
                        match show_launcher_scope(&mut palette, main_ui, request.value["mode"].as_str().unwrap(), request.value["query"].as_str(), request.context.received_at()) {
                            Ok(()) => json!({"ok":true,"result":{"state":"opened"}}),
                            Err(error) => json!({"ok":false,"error":error}),
                        }
                    } else if request.value["mode"] == "analytics" {
                        if let Some(target)=&management_ui {
                            target.action(Action::OpenDashboard);
                            main_ui.action(Action::Key("cancelOperation:".into()));
                            palette.session.visible=false;
                            json!({"ok":true,"result":{"state":"opened"}})
                        } else { json!({"ok":false,"error":"Settings window is unavailable."}) }
                    } else if request.value["mode"] == "settings" {
                        if let Some(ui) = &management_ui {
                            match management.show(
                                request.value["query"].as_str(),
                                request.context.received_at(),
                                ui,
                            ) {
                                Ok(()) => json!({"ok":true,"result":{"state":"opened"}}),
                                Err(error) => json!({"ok":false,"error":error}),
                            }
                        } else {
                            json!({"ok":false,"error":"Settings window is unavailable."})
                        }
                    } else if request.value["mode"].as_str().is_some_and(|mode|utilities::kind(mode).is_some()) {
                        match utilities::show(&mut state,&mut palette,main_ui,request.value["mode"].as_str().unwrap(),request.value["query"].as_str(),request.context.received_at()){Ok(())=>json!({"ok":true,"result":{"state":"opened"}}),Err(error)=>json!({"ok":false,"error":error})}
                    } else if request.value["mode"]
                        .as_str()
                        .is_none_or(|m| crate::system_search::Scope::parse(m).is_some())
                    {
                        match palette.show(
                            request.value["query"].as_str(),
                            request.context.received_at(),
                            main_ui,
                        ) {
                            Ok(()) => {
                                palette.system.enter(request.value["mode"].as_str().and_then(crate::system_search::Scope::parse).unwrap_or(crate::system_search::Scope::MainTabs));
                                capture_inline_query(&state, &mut palette, main_ui);
                                json!({"ok":true,"result":{"state":"opened"}})
                            }
                            Err(error) => json!({"ok":false,"error":error}),
                        }
                    } else {
                        json!({"ok":false,"error":"This mode is not yet available in the Rust development build."})
                    }
                } else {
                    dispatch(
                        &mut state,
                        &mut palette,
                        &request.value,
                        &observer,
                        ui.is_none(),
                    )
                };
                if let Some(ui) = &ui {
                    palette.publish(&state, ui);
                }
                if let Some(ui) = &management_ui {
                    management.publish(&state, ui);
                }
                if request.value["op"] == "status" {
                    if let Some(ui) = &ui {
                        value["result"]["nativeUI"] = palette.status(ui);
                    }
                    if let Some(ui) = &management_ui {
                        value["result"]["managementUI"] = management.status(ui);
                    }
                }
                if value["ok"] == true
                    && matches!(
                        request.value["op"].as_str(),
                        Some("analytics.settings" | "analytics.reset")
                    )
                {
                    match state.analytics.client().flush_async() {
                        Ok(done) => {
                            let reset = request.value["op"] == "analytics.reset";
                            analytics_responses.push((request, done, reset));
                            continue;
                        }
                        Err(error) => {
                            value = analytics_unconfirmed();
                            value["reason"] = json!(error);
                        }
                    }
                }
                quit = request.value["op"] == "quit";
                if quit {
                    palette.retire_creation();
                    management.retire_creation();
                    if let Some(writer) = &state.writer {
                        writer.stop();
                    }
                    state.cancel_configuration_reply(&mut shutdown_deliveries);
                }
                let delivered = request.respond(value);
                if quit {
                    let _ = delivered.recv_timeout(Duration::from_secs(3));
                }
            }
        }
        if let Some(ui) = &ui {
            system::poll_native(&mut state,&mut palette,ui);
            palette.publish(&state, ui);
        }
        if let Some(ui) = &management_ui {
            system::poll_native(&mut state,&mut management,ui);
            management.publish(&state, ui);
        }
    }
    palette.retire_creation();
    management.retire_creation();
    if let Some(writer) = &state.writer {
        writer.stop();
    }
    state.cancel_configuration_reply(&mut shutdown_deliveries);
    let completed = state.execution.shutdown();
    shutdown_deliveries.extend(finish_executions(
        &mut state,
        &mut palette,
        &mut management,
        completed,
    ));
    let reply_deadline = Instant::now() + Duration::from_secs(3);
    for (request, done, reset) in analytics_responses {
        let response =
            match done.recv_timeout(reply_deadline.saturating_duration_since(Instant::now())) {
                Ok(()) => analytics_response(&state.analytics.client(), reset),
                Err(_) => analytics_unconfirmed(),
            };
        shutdown_deliveries.push(request.respond(response));
    }
    for delivery in shutdown_deliveries {
        let _ = delivery.recv_timeout(reply_deadline.saturating_duration_since(Instant::now()));
    }
    state.diagnostics.finish();
    discovery.stop();
    observer.stop();
    server.stop();
    if quit
        || ui.as_ref().is_some_and(|ui| ui.stop_requested())
        || management_ui.as_ref().is_some_and(|ui| ui.stop_requested())
    {
        Ok(json!({"ok":true,"result":{"state":"stopped"}}))
    } else {
        Err("Control server stopped unexpectedly.".into())
    }
}
fn analytics_response(client: &crate::analytics::Client, reset: bool) -> Value {
    let mut response = json!({"ok":client.storage_error().is_none(),"result":client.snapshot()});
    if !reset {
        response["storageError"] = json!(client.storage_error());
    }
    response
}
fn analytics_unconfirmed() -> Value {
    json!({"ok":false,"error":"Analytics settings were applied in memory; storage completion is unconfirmed.","applied":true,"persistence":"unknown"})
}
fn handle_hotkey(
    state: &mut SharedState,
    window: &mut WindowRuntime,
    ui: &Arc<Connection>,
    event: crate::hotkeys::RoutedPress,
) {
    let _performance = crate::performance_trace::shared()
        .scope(crate::performance_trace::Stage::HotkeyHandler);
    let Some(keyboard) = state.keyboard.clone() else {
        return;
    };
    if ui.stop_requested()
        || !keyboard.running()
        || keyboard.paused()
        || event
            .hook_epoch
            .is_some_and(|epoch| epoch != keyboard.hook_epoch() || !keyboard.hooks_available())
        || keyboard.recording()
        || event.recording_epoch != keyboard.recording_epoch()
        || event.ui_epoch != ui.latest_epoch()
        || event.ui_intent != ui.routing_revision()
        || event.ui_dismissal != ui.latest_dismissal()
        || event.received.elapsed() > Duration::from_secs(5)
    {
        keyboard.note_dropped();
        return;
    }
    let press = &event.press;
    if press.generation == 0
        || press.generation != state.configuration.keyboard_generation()
        || press.generation != keyboard.accepted_generation()
    {
        // Registry routes can switch inside commit before Carbon returns and
        // before the main-thread ACK reaches either client or state owner.
        // Retain one event while a writer is active; final generation checks
        // still run after adoption, so obsolete routes cannot execute.
        if state.config_busy() {
            if state.deferred_hotkey.replace(event).is_some() {
                keyboard.note_dropped();
            }
        } else {
            keyboard.note_dropped();
        }
        return;
    }
    let key = if press.action == "@launcher" {
        Some(&state.configuration.current().launcher_hotkey)
    } else if press.action == "@clipboard" {
        Some(&state.configuration.current().clipboard_hotkey)
    } else {
        state
            .configuration
            .current()
            .commands
            .iter()
            .find(|c| c.id == press.action && c.enabled != Some(false))
            .and_then(|c| c.hotkey.as_ref())
    };
    if key.and_then(|k| rustcast_core::shortcut(k).ok()).as_ref() != Some(&press.canonical) {
        keyboard.note_dropped();
        return;
    }
    if press.action == "@launcher" {
        window.input_intent = ui.global_action();
        window.dismissal = ui.latest_dismissal();
        if window.session.visible && window.results.token().is_none()
            && !window.session.clipboard_mode && window.session.launcher_scope.is_none()
            && window.utility.mode.is_none() && window.session.library.is_none()
            && window.system.scope == crate::system_search::Scope::MainTabs
        {
            window.retire_creation();
            window.field_preparation.cancel();
            window.field_request = None;
            window.editing.cancel_all();
            window.session.actions_open = false;
            window.session.visible = false;
            window.visibility_intent = Some(event.received);
            ui.cancel_open_hide();
        } else if let Err(error) = window.show(Some(""), event.received, ui) {
            window.message = error;
        } else if !ui.is_management {
            let _ = system::consent(state, window, ui);
        }
        return;
    }
    if press.action == "@clipboard" {
        window.input_intent = ui.global_action();
        window.dismissal = ui.latest_dismissal();
        if let Err(error)=show_clipboard(state,window,ui,event.received){window.message=error;}
        return;
    }
    if state.config_busy() || state.config_dirty {
        if state.deferred_hotkey.replace(event).is_some() {
            keyboard.note_dropped();
        }
        return;
    }
    if state.reload_watched().is_err() {
        if state.config_busy() {
            state.deferred_hotkey = Some(event);
        } else {
            keyboard.note_dropped();
        }
        return;
    }
    // Revalidation may have adopted a new generation. Never resolve this ID as
    // an alias or substitute whatever happens to be the selected result row.
    if press.generation != state.configuration.keyboard_generation() {
        keyboard.note_dropped();
        return;
    }
    let Some(command) = state
        .configuration
        .current()
        .commands
        .iter()
        .find(|c| c.id == press.action && c.enabled != Some(false))
        .cloned()
    else {
        keyboard.note_dropped();
        return;
    };
    if state.execution.any(|c|matches!(&c.destination,ExecutionDestination::Native{ui:owner,..} if owner.ptr_eq(&Arc::downgrade(ui)))) {
        window.notify("A command is already running in this window.".into(),false);return;
    }
    window.input_intent = ui.global_action();
    window.dismissal = ui.latest_dismissal();
    window.retire_creation();
    window.editing.cancel_all();
    window.field_preparation.cancel();
    window.field_request = None;
    if let Some(token) = window.results.token() {
        ui.cancel_result(token);
    }
    window.results.clear();
    window.session.actions_open = false;
    window.native_source = "hotkey";
    if let Err(error) = admit_native_command(state, window, ui, ui.latest_dismissal(), command) {
        window.message = error.clone();
        window.notify(error, false);
    }
}
fn uuid() -> String {
    objc2::rc::autoreleasepool(|_| objc2_foundation::NSUUID::new().UUIDString().to_string())
}
fn saved_warning(receipt: crate::profile::SaveReceipt) -> Option<String> {
    receipt.warning.map(|warning| {
        if receipt.durable {
            warning
        } else {
            format!("Saved, but disk confirmation failed: {warning}")
        }
    })
}
fn start_configuration_edit(
    state: &mut SharedState,
    window: &mut WindowRuntime,
    ui: &Arc<Connection>,
    deadline: Instant,
    work: impl FnOnce(
            &mut crate::profile::ConfigStore,
            &mut crate::actions::Editing,
            &crate::app_choice::Choices,
            crate::execution::Live,
        ) -> Result<(Option<String>, ConfigAfter), String>
        + Send
        + 'static,
) -> Result<(), String> {
    let origin = Arc::new(ConfigOrigin {
        draft_live: window.editing.liveness_guard(),
        ui: Arc::downgrade(ui),
        epoch: ui.latest_epoch(),
        intent: ui.latest_intent(),
        dismissal: ui.latest_dismissal(),
        deadline,
        before: window.editing.view(),
    });
    let check = origin.clone();
    let live: crate::execution::Live = Arc::new(move || check.storage_live());
    let work_guard = origin.clone();
    let work_live: crate::execution::Live =
        Arc::new(move || work_guard.storage_live() && work_guard.clocks());
    let mut editing = window.editing.clone();
    let apps = state.app_choices.clone();
    state
        .writer
        .as_mut()
        .ok_or("Configuration writer unavailable.")?
        .start(&state.configuration, live, false, move |store| {
            let (warning, after, error) = match work(store, &mut editing, &apps, work_live) {
                Ok((warning, after)) => (warning, after, None),
                Err(error) => {
                    editing.fail(error.clone());
                    (None, ConfigAfter::None, Some(error))
                }
            };
            editing.set_saving(false);
            Ok(ConfigPayload {
                editing: Some(editing),
                warning,
                error,
                after,
                response: None,
            })
        })?;
    window.editing.set_saving(true);
    state.config_context = Some(ConfigContext::Native(origin));
    Ok(())
}
fn finish_configuration(
    state: &mut SharedState,
    palette: &mut WindowRuntime,
    management: &mut WindowRuntime,
) {
    let completed = state.writer.as_mut().and_then(|w| w.take());
    if let Some(completed) = completed {
        let context = state.config_context.take();
        let uncertain = completed.uncertain
            || (completed.published || completed.retention.is_some())
                && match &completed.result {
                    Err(_) => true,
                    Ok(p) => p.error.is_some(),
                };
        state.configuration = completed.store;
        if uncertain {
            state.config_dirty = true;
            state.configuration.require_reconciliation();
            state.configuration.last_error =
                Some("Configuration may have been saved. Reload it before trying again.".into());
        }
        let accepted = !uncertain && completed.result.as_ref().is_ok_and(|p| p.error.is_none());
        if accepted {
            if let Some(services) = &state.services {
                services.clipboard_capture.publish(
                    &state.configuration.current().clipboard,
                    state.clipboard.as_ref().is_none_or(|s|s.paused),
                );
            }
            if let Some(writer) = &state.writer { writer.retention_phase().store(false, std::sync::atomic::Ordering::SeqCst); }
        }
        if let Some(store) = &mut state.clipboard {
            if let Some(receipt) = completed.retention {
                if !receipt.durable {
                    store.warning = receipt.warning;
                    store.cleanup_error = Some("Clipboard publication durability is unconfirmed; assets retained.".into());
                }
            }
            if accepted {
                // Includes explicit unchanged reload: retry cleanup once here,
                // rather than waiting for the periodic minute or each UI tick.
                // Re-read under the existing lock; never adopt a worker's stale
                // in-memory history over captures accepted in the meantime.
                let confirm = store.warning.is_some();
                if let Err(error) = store.reconcile_settings(&state.configuration.current().clipboard, confirm, || true) {
                    store.cleanup_error = Some(error);
                }
            }
        }
        if let Some(keyboard) = &state.keyboard {
            keyboard.confirm_keywords(if uncertain {
                0
            } else {
                state.configuration.keyboard_generation()
            });
        }
        palette.sync(state);
        management.sync(state);
        match context {
            #[cfg(feature = "test-tools")]
            Some(ConfigContext::Owned(request))=>{
                let result=match completed.result {
                    Ok(payload)=>owned_catalog_adoption(&state.configuration,uncertain,payload),
                    Err(error)=>Err(error),
                };
                let _=request.response.try_send(result);
            }
            Some(ConfigContext::Cli(request)) => {
                let mut response = match completed.result {
                    Ok(payload) => payload.response.unwrap_or_else(|| json!({"ok":true})),
                    Err(error) => json!({"ok":false,"error":error}),
                };
                response["configurationWrite"] =
                    json!({"published":completed.published,"outcomeUncertain":completed.uncertain});
                response["keyboardActivationPending"] = json!(state.configuration.needs_effects());
                response["configurationWarning"] = json!(state.configuration.last_error);
                response["configurationReconciliationRequired"] =
                    json!(uncertain || state.configuration.needs_effects());
                response["clipboardWarning"] = json!(state.clipboard.as_ref().and_then(|s|s.warning.as_ref()));
                response["clipboardCleanupError"] = json!(state.clipboard.as_ref().and_then(|s|s.cleanup_error.as_ref()));
                if uncertain {
                    response["error"] = json!(format!(
                        "Configuration may have been saved. Reload it before trying again. {}",
                        response["error"].as_str().unwrap_or("")
                    ));
                }
                let _ = request.respond(response);
            }
            Some(ConfigContext::Native(origin)) => {
                if let Some(ui) = origin.ui.upgrade() {
                    let window = if ui.is_management {
                        &mut *management
                    } else {
                        &mut *palette
                    };
                    let mut current = window.editing.view();
                    if let Some(view) = current.as_mut() {
                        view.saving = false;
                    }
                    let same = current == origin.before;
                    if same {
                        window.editing.set_saving(false);
                    }
                    if same && origin.ownership() {
                        match completed.result {
                            Ok(payload) => {
                                if let Some(mut editing) = payload.editing {
                                    let resumes_owned_input = origin
                                        .before
                                        .as_ref()
                                        .and_then(|v| v.suspended_input_token)
                                        .is_some_and(|parent| {
                                            !ui.editor_cancelled(parent)
                                                && editing.view().is_some_and(|v| {
                                                    v.resumed_input_from == Some(parent)
                                                })
                                        });
                                    // Closing/acknowledging a saved form survives slow
                                    // owner delivery. Opening a different form does not.
                                    if !origin.live()
                                        && !resumes_owned_input
                                        && editing.token().is_some()
                                        && editing.token()
                                            != origin.before.as_ref().map(|v| v.token)
                                    {
                                        editing.cancel_all();
                                    }
                                    window.editing = editing;
                                }
                                window.message = [
                                    payload.warning.unwrap_or_default(),
                                    payload.error.unwrap_or_default(),
                                ]
                                .into_iter()
                                .filter(|v| !v.is_empty())
                                .collect::<Vec<_>>()
                                .join("\n");
                                if uncertain {
                                    window.message=format!("Changes may have been saved. Check configuration before trying again. {}",window.message);
                                }
                                if origin.live() {
                                    let result = match payload.after {
                                        ConfigAfter::None => Ok(None),
                                        ConfigAfter::Navigate { kind, value } => {
                                            window.session.actions_open = false;
                                            let r = window.session.navigate(&kind, &value);
                                            ui.adopt_query(&window.session.query, window.epoch);
                                            r.map(|_| None)
                                        }
                                        ConfigAfter::Named {
                                            name,
                                            origin: parent,
                                        } => {
                                            let r = perform_named(
                                                state,
                                                window,
                                                &name,
                                                &ui,
                                                None,
                                                origin.dismissal,
                                                origin.deadline,
                                            );
                                            window.editing.bind_origin_inline(parent);
                                            r
                                        }
                                        ConfigAfter::Select(id) => {
                                            let row =
                                                window.session.rows().position(|c| c.id == id);
                                            window.session.select_row(row);
                                            Ok(None)
                                        }
                                    };
                                    if let Err(error) = result {
                                        window.editing.fail(error.clone());
                                        window.message = error;
                                    }
                                }
                            }
                            Err(error) => {
                                window.editing.fail(error.clone());
                                window.message = error;
                            }
                        }
                    } else if same && completed.published {
                        window.editing.cancel_all();
                    }
                    window.sync(state);
                }
            }
            Some(ConfigContext::WatchThenCli(request)) => state.admit_cli_reload(request),
            Some(ConfigContext::Watch) | None => {}
        }
    }
    if state.config_dirty
        && !state.config_busy()
        && state
            .queue_reload(
                ConfigContext::Watch,
                Arc::new(|| true),
                state.configuration.needs_effects(),
            )
            .is_ok()
    {
        state.config_dirty = false;
    }
}
fn execution_identity(a: &rustcast_core::Command, b: &rustcast_core::Command) -> bool {
    a.id == b.id
        && a.kind == b.kind
        && a.target == b.target
        && a.arguments == b.arguments
        && a.timeout == b.timeout
        && a.input_prompt == b.input_prompt
        && a.application_id == b.application_id
        && b.enabled != Some(false)
}
fn origin_valid_parts(
    config: &crate::profile::ConfigStore,
    apps: &[rustcast_core::Command],
    generation: u64,
    context: &ExecutionContext,
) -> bool {
    if context.file != config.file {
        return false;
    }
    if let Some(key) = &context.requested_key {
        let commands = &config.current().commands;
        if commands
            .iter()
            .any(|c| c.enabled == Some(false) && name_matches(c, key))
        {
            return false;
        }
        let saved = commands.iter().find(|c| name_matches(c, key));
        match (&context.origin, saved) {
            (ExecutionOrigin::Saved, Some(c)) if execution_identity(&context.command, c) => {}
            (ExecutionOrigin::Discovered(_) | ExecutionOrigin::Service(_), None) if command_key(&context.command.id) == *key => {}
            (ExecutionOrigin::Utility,None) if command_key(&context.command.id)==*key=>{}
            _ => return false,
        }
    }
    match context.origin {
        ExecutionOrigin::Clipboard(_) => false,
        ExecutionOrigin::Utility=>utilities::recognizes(&context.command)&&context.command.target.starts_with("utility.history:"),
        ExecutionOrigin::Service(pin)=>pin==config.revision && service_file(&context.command),
        ExecutionOrigin::Saved => {
            config
                .current()
                .commands
                .iter()
                .any(|c| execution_identity(&context.command, c))
                && (context.command.kind != "app"
                    || !crate::applications::disabled_target(
                        &config.current().commands,
                        &context.command.target,
                    ))
        }
        ExecutionOrigin::Discovered(pin) => {
            pin == generation
                && apps.iter().any(|c| execution_identity(&context.command, c))
                && crate::applications::available(&config.current().commands, &context.command)
        }
    }
}
fn origin_valid(state: &SharedState, context: &ExecutionContext) -> bool {
    if let ExecutionOrigin::Clipboard(revision)=context.origin {return state.clipboard.as_ref().is_some_and(|s|s.revision==revision&&s.items.iter().any(|c|format!("clipboard:{}",c.id)==context.command.id));}
    if matches!(context.origin,ExecutionOrigin::Utility){return context.file==state.configuration.file&&state.utility.command(&command_key(&context.command.id)).is_some_and(|command|execution_identity(command,&context.command));}
    origin_valid_parts(
        &state.configuration,
        &state.applications,
        state.applications_revision,
        context,
    )
}
fn service_file(command:&rustcast_core::Command)->bool {command.kind=="path" && command.id==format!("system.file.{:x}",<sha2::Sha256 as sha2::Digest>::digest(command.target.as_bytes())) && command.enabled!=Some(false)}
fn resolve_command(
    state: &SharedState,
    window: &WindowRuntime,
    id: &str,
) -> Result<(rustcast_core::Command, ExecutionOrigin), String> {
    let id = command_key(id);
    if state.configuration.current().commands.iter().any(|c| {
        c.enabled == Some(false)
            && (command_key(&c.id) == id || c.aliases.iter().any(|a| command_key(a) == id))
    }) {
        return Err("Command is disabled.".into());
    }
    let saved = state
        .configuration
        .current()
        .commands
        .iter()
        .find(|c| command_key(&c.id) == id || c.aliases.iter().any(|a| command_key(a) == id));
    let (command, origin) = match saved {
        Some(c) => (c.clone(), ExecutionOrigin::Saved),
        None if state.utility.command(&id).is_some() =>(state.utility.command(&id).unwrap().clone(),ExecutionOrigin::Utility),
        None => (
            (0..window.session.catalog.len())
                .map(|i| window.session.catalog.command(i))
                .find(|c| command_key(&c.id) == id || c.aliases.iter().any(|a|command_key(a)==id)).cloned().or_else(||state.system.command(&id).filter(service_file))
                .ok_or("Unknown command.")?,
            ExecutionOrigin::Discovered(state.applications_revision),
        ),
    };
    if command.enabled == Some(false)
        || (command.kind == "app"
            && crate::applications::disabled_target(
                &state.configuration.current().commands,
                &command.target,
            ))
    {
        return Err("Command is disabled.".into());
    }
    let origin=if service_file(&command) && matches!(origin,ExecutionOrigin::Discovered(_)){ExecutionOrigin::Service(state.configuration.revision)}else{origin};
    Ok((command, origin))
}
fn command_key(value: &str) -> String {
    objc2::rc::autoreleasepool(|_| {
        objc2_foundation::NSString::from_str(value)
            .lowercaseString()
            .precomposedStringWithCanonicalMapping()
            .to_string()
    })
}
fn name_matches(command: &rustcast_core::Command, key: &str) -> bool {
    command_key(&command.id) == key || command.aliases.iter().any(|a| command_key(a) == key)
}
fn prepare_cli_open(
    command: rustcast_core::Command,
    values: Value,
    clipboard: Option<&str>,
) -> Result<crate::workspace::OpenRequest, String> {
    objc2::rc::autoreleasepool(|_| {
        if let Some(prompt) = &command.input_prompt {
            use objc2_foundation::{NSCharacterSet, NSString};
            if NSString::from_str(values["input"].as_str().unwrap_or("")).stringByTrimmingCharactersInSet(&NSCharacterSet::whitespaceAndNewlineCharacterSet()).is_empty() {return Err(format!("Enter {prompt}."));}
        }
        let kind = match command.kind.as_str() {
            "app" => crate::workspace::Kind::App,
            "url" => crate::workspace::Kind::Url,
            "path" => crate::workspace::Kind::Path,
            _ => return Err("Command kind is not available.".into()),
        };
        let target = if matches!(command.kind.as_str(), "url" | "snippet") {
            let plan =
                crate::templates::Plan::parse(&command, crate::templates::Limits::application())?;
            let data = values
                .as_object()
                .ok_or("Input values must be strings.")?
                .iter()
                .map(|(k, v)| (k.clone(), v.as_str().unwrap_or("").to_string()))
                .collect::<Vec<_>>();
            plan.render(&data, &crate::templates::Context::current(clipboard))?
                .0
        } else {
            command.target
        };
        Ok(crate::workspace::OpenRequest {
            kind,
            command_id: command.id,
            title: command.title,
            target,
            application_id: command.application_id,
        })
    })
}
fn owner_grant(
    events: mpsc::SyncSender<HostEvent>,
    token: String,
    live: crate::execution::Live,
    deadline: impl Into<Option<Instant>>,
    hide: bool,
) -> Arc<dyn Fn() -> Result<(), String> + Send + Sync> {
    owner_native_grant(events, token, live, deadline, if hide { 1 } else { 0 })
}
fn owner_native_grant(
    events: mpsc::SyncSender<HostEvent>,
    token: String,
    hook_live: crate::execution::Live,
    deadline: impl Into<Option<Instant>>,
    mode: u8,
) -> Arc<dyn Fn() -> Result<(), String> + Send + Sync> {
    let deadline = deadline.into();
    Arc::new(move || {
        let (reply, receive) = mpsc::sync_channel(1);
        events
            .try_send({
                let commit = crate::execution::CommitRequest {
                    token: token.clone(),
                    reply,
                };
                match mode {
                    0 => HostEvent::OpenCommit(commit),
                    1 => HostEvent::OpenHide(commit),
                    2 => HostEvent::PasteRead(commit),
                    3 => HostEvent::PasteCommit(commit),
                    _ => HostEvent::PasteHide(commit),
                }
            })
            .map_err(|_| "Execution owner is busy or stopped.".to_string())?;
        loop {
            if !hook_live() || crate::deadline::expired(deadline) {
                return Err("Request expired or was cancelled.".into());
            }
            match receive.recv_timeout(crate::deadline::poll(deadline)) {
                Ok(value) => return value,
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(_) => {
                    return Err(if mode < 2 {
                        "Execution owner stopped before opening."
                    } else {
                        "Execution owner stopped before snippet paste."
                    }
                    .into())
                }
            }
        }
    })
}
fn handle_open_grant(
    state: &mut SharedState,
    palette: &mut WindowRuntime,
    management: &mut WindowRuntime,
    commit: crate::execution::CommitRequest,
    hide: bool,
) {
    let current = state.reload_watched().is_ok();
    let context = current
        .then(|| {
            state
                .execution
                .find_open(|c| c.trace == commit.token && origin_valid(state, c))
        })
        .flatten();
    let valid = context.is_some_and(|context| {
        if let ExecutionDestination::Native {
            ui,
            epoch,
            dismissal,
            intent,
            serial,
            ..
        } = &context.destination
        {
            let Some(ui) = ui.upgrade() else { return false };
            let key = crate::connected::HideKey {
                trace: context.trace.clone(),
                epoch: *epoch,
                dismissal: *dismissal,
                intent: *intent,
                serial: *serial,
            };
            if !ui.open_hide_live(&key) {
                return false;
            }
            let window = if ui.is_management {
                &mut *management
            } else {
                &mut *palette
            };
            if window.execution_serial != *serial || window.epoch != *epoch {
                return false;
            }
            if hide {
                window
                    .execution_effect
                    .get_or_insert((*intent, window.session.visible));
                window.session.visible = false;
                window.dashboard.retire();
                window.session.actions_open = false;
            }
            true
        } else {
            !hide
        }
    });
    let _ = commit.reply.try_send(if valid {
        Ok(())
    } else {
        Err("Command changed or the request expired before opening.".into())
    });
}
fn handle_paste_grant(
    state: &mut SharedState,
    palette: &mut WindowRuntime,
    commit: crate::execution::CommitRequest,
    phase: u8,
) {
    let valid = (|| -> Result<(), String> {
        state.reload_watched()?;
        let context = state
            .execution
            .find_open(|c| {
                c.trace == commit.token && (c.command.kind == "snippet"||utilities::recognizes(&c.command)) && origin_valid(state, c)
            })
            .ok_or("Command changed before snippet paste.")?;
        if matches!(context.origin,ExecutionOrigin::Clipboard(_)) && context.command.target=="copy-only" {return Ok(())}
        let Some(origin) = &context.snippet_hide else {
            return if phase == 0 {
                Ok(())
            } else {
                Err("Snippet window is unavailable.".into())
            };
        };
        if !origin.live() {
            return Err("Snippet target intent changed.".into());
        }
        let ui = origin.ui.upgrade().ok_or("Snippet window stopped.")?;
        if ui.is_management || palette.epoch != origin.epoch {
            return Err("Snippet window changed.".into());
        }
        if phase == 0 {
            return Ok(());
        }
        let mut slot = origin.slot.lock().unwrap_or_else(|p| p.into_inner());
        if slot.is_none() {
            if phase == 2 {
                return Err("Snippet hide was not authorized.".into());
            }
            let serial = if let Some(serial) = origin.native_serial {
                if palette.execution_serial != serial {
                    return Err("Snippet execution changed.".into());
                }
                serial
            } else {
                palette
                    .execution_serial
                    .checked_add(1)
                    .ok_or("Execution sequence exhausted.")?
            };
            let key = crate::connected::HideKey {
                trace: context.trace.clone(),
                epoch: origin.epoch,
                dismissal: origin.dismissal,
                intent: origin.intent,
                serial,
            };
            *slot = Some(ui.arm_open_hide(key)?);
            palette.execution_serial = serial;
        }
        let token = slot.as_ref().expect("authorized hide");
        if !ui.open_hide_live(&token.key) || palette.execution_serial != token.key.serial {
            return Err("Snippet window changed before copying.".into());
        }
        if phase == 2 {
            palette
                .execution_effect
                .get_or_insert((origin.intent, palette.session.visible));
            palette.session.visible = false;
            palette.session.actions_open = false;
        }
        Ok(())
    })();
    let _ = commit.reply.try_send(valid);
}
#[allow(clippy::too_many_arguments)]
fn start_snippet(
    state: &mut SharedState,
    command: rustcast_core::Command,
    values: Value,
    destination: ExecutionDestination,
    origin: ExecutionOrigin,
    requested_key: Option<String>,
    live: crate::execution::Live,
    deadline: Option<Instant>,
    hide_origin: Option<SnippetHideOrigin>,
) -> Result<(), String> {
    let Some(services) = state.services.clone() else {
        let error = "Main-thread service is unavailable.";
        if let ExecutionDestination::Cli(request) = destination {
            let _ = request.respond(json!({"ok":false,"error":error}));
        }
        return Err(error.into());
    };
    let clipboard_prepared=if matches!(origin,ExecutionOrigin::Clipboard(_)) {
        match state.clipboard.as_ref().ok_or_else(||String::from("Clipboard history unavailable.")).and_then(|s|s.payload(command.id.strip_prefix("clipboard:").unwrap_or(""))) {
            Ok(payload)=>Some(crate::paste::Prepared::clipboard(payload,values["_copyOnly"]==true)),
            Err(error)=>{if let ExecutionDestination::Cli(request)=destination {let _=request.respond(json!({"ok":false,"error":error}));}return Err(error)}
        }
    }else{None};
    let trace = uuid();
    let logged_id =
        crate::activity::Activity::log_id(analytics_id(&command), &values, &state.configuration.paths.home);
    let permit = crate::main_services::Permit::default();
    let revoked = permit.live();
    let hide = hide_origin
        .as_ref()
        .map(|o| o.slot.clone())
        .unwrap_or_default();
    let dispatch = match &destination {
        ExecutionDestination::Cli(request) => request.context.paste_dispatch(),
        _ => Arc::new(crate::paste::Dispatch::default()),
    };
    let deadline_copy = deadline;
    let events = state.events.clone();
    let request_trace = trace.clone();
    let source = destination.source();
    let history_operation=matches!(origin,ExecutionOrigin::Clipboard(_));
    let context = ExecutionContext {
        capture_result: None,
        initial_result: None,
        analytics: if history_operation {None}else{crate::analytics::Span::new(
            state.analytics.client(),
            analytics_id(&command),
            source,
        )},
        snippet_hide: hide_origin,
        destination,
        command: command.clone(),
        file: state.configuration.file.clone(),
        trace: trace.clone(),
        logged_id: logged_id.clone(),
        log_secrets: log_secrets(&values),
        started: Instant::now(),
        origin,
        requested_key,
        _services: permit,
    };
    let copy_dispatch = dispatch.clone();
    let cli = source == "cli";
    match state
        .execution
        .admit_snippet(cli, live, deadline, dispatch, context, move |live| {
            let prepare = (|| -> Result<_, String> {
                if let Some(prepared)=clipboard_prepared {return Ok(prepared)}
                let clipboard = if crate::templates::needs_clipboard(&command.target) {
                    owner_native_grant(
                        events.clone(),
                        request_trace.clone(),
                        live.clone(),
                        deadline_copy,
                        2,
                    )()?;
                    services.clipboard_for_snippet(
                        deadline_copy,
                        live.clone(),
                        revoked.clone(),
                        copy_dispatch.clone(),
                    )?
                } else {
                    None
                };
                crate::paste::Prepared::render(&command, &values, clipboard.as_deref())
            })();
            let prepared = match prepare {
                Ok(p) => p,
                Err(e) => return crate::paste::Outcome::failure(e, false),
            };
            let grant_events = events.clone();
            let grant_trace = request_trace.clone();
            let grant_live = live.clone();
            services.paste().execute(
                prepared,
                hide,
                live,
                deadline_copy,
                revoked,
                copy_dispatch,
                move |hide| {
                    owner_native_grant(
                        grant_events.clone(),
                        grant_trace.clone(),
                        grant_live.clone(),
                        deadline_copy,
                        if hide { 4 } else { 3 },
                    )()
                },
            )
        }) {
        Ok(_) => {
            state.open_validation_stamp = None;
            if !history_operation {state.activity.record(&trace, &logged_id, source, None);}
            Ok(())
        }
        Err((error, mut context)) => {
            if let Some(span) = context.analytics.take() {
                span.finish(false, crate::analytics::failure_code(&error));
            }
            if let ExecutionDestination::Cli(request) = context.destination {
                let _ = request.respond(json!({"ok":false,"error":error}));
            }
            Err(error)
        }
    }
}
fn start_native_snippet(
    state: &mut SharedState,
    window: &mut WindowRuntime,
    ui: &Arc<Connection>,
    dismissal: u64,
    command: rustcast_core::Command,
    values: Value,
    input_token: Option<u64>,
) -> Result<(), String> {
    let serial = window
        .execution_serial
        .checked_add(1)
        .ok_or("Execution sequence exhausted.")?;
    let hide_ui = if ui.is_management {
        ui.peer().ok_or("Palette window is unavailable.")?
    } else {
        ui.clone()
    };
    let hide = SnippetHideOrigin::capture(&hide_ui, (!ui.is_management).then_some(serial));
    let check = hide.clone();
    let weak = Arc::downgrade(ui);
    let epoch = window.epoch;
    let intent = window.input_intent;
    let live: crate::execution::Live = Arc::new(move || {
        check.live()
            && weak.upgrade().is_some_and(|ui| {
                !ui.stop_requested()
                    && ui.latest_epoch() == epoch
                    && ui.latest_intent() == intent
                    && ui.latest_dismissal() == dismissal
                    && input_token.is_none_or(|t| !ui.editor_cancelled(t))
            })
    });
    let destination = ExecutionDestination::Native {
        source: window.native_source,
        ui: Arc::downgrade(ui),
        epoch,
        dismissal,
        intent,
        serial,
        input_token,
        result_origin: None,
    };
    let origin=if command.id.starts_with("clipboard:"){ExecutionOrigin::Clipboard(state.clipboard.as_ref().map_or(0,|s|s.revision))}else{ExecutionOrigin::Saved};
    start_snippet(
        state,
        command,
        values,
        destination,
        origin,
        None,
        live,
        native_open_deadline(state),
        Some(hide),
    )?;
    window.execution_serial = serial;
    window.execution_effect = None;
    window.message.clear();
    window.session.actions_open = false;
    Ok(())
}
fn start_cli_open(
    state: &mut SharedState,
    command: rustcast_core::Command,
    origin: ExecutionOrigin,
    values: Value,
    request: crate::control::Envelope,
) {
    let trace = uuid();
    let token = trace.clone();
    let events = state.events.clone();
    let request_context = request.context.clone();
    // Leave time to publish an honest native timeout before transport expiry.
    let deadline = request_context
        .deadline()
        .checked_sub(Duration::from_millis(100))
        .unwrap_or(request_context.deadline());
    let dispatch = request_context.workspace_dispatch();
    let live_context = request_context.clone();
    let live: crate::execution::Live = Arc::new(move || live_context.is_live());
    let hook_live = live.clone();
    let logged_id =
        crate::activity::Activity::log_id(analytics_id(&command), &values, &state.configuration.paths.home);
    let prepared = command.clone();
    let requested_key = request.value["id"].as_str().map(command_key);
    let services_permit = crate::main_services::Permit::default();
    let revoked = services_permit.live();
    let services = state.services.clone();
    let context = ExecutionContext {
        capture_result: None,
        initial_result: None,
        analytics: crate::analytics::Span::new(
            state.analytics.client(),
            analytics_id(&command),
            "cli",
        ),
        snippet_hide: None,
        _services: services_permit,
        destination: ExecutionDestination::Cli(request),
        command,
        file: state.configuration.file.clone(),
        trace: trace.clone(),
        logged_id: logged_id.clone(),
        log_secrets: log_secrets(&values),
        started: Instant::now(),
        origin,
        requested_key,
    };
    let mode = if state.fixture {
        #[cfg(feature = "test-tools")]
        {
            crate::execution::WorkspaceMode::Fixture(state.configuration.paths.home.clone())
        }
        #[cfg(not(feature = "test-tools"))]
        {
            unreachable!("Fixture runtime requires test tools")
        }
    } else {
        crate::execution::WorkspaceMode::Production
    };
    let grant = owner_grant(events, token, hook_live, deadline, false);
    let capture_grant = grant.clone();
    let capture_live = live.clone();
    let capture_dispatch = dispatch.clone();
    match state.execution.admit_open_prepared_tracked(
        Box::new(move || {
            let needs_clipboard =
                prepared.kind == "url" && crate::templates::needs_clipboard(&prepared.target);
            let clipboard = if needs_clipboard {
                capture_grant()?;
                Some(
                    services
                        .as_ref()
                        .ok_or("Main-thread clipboard service is unavailable.")?
                        .clipboard(deadline, capture_live, revoked, capture_dispatch)?,
                )
            } else {
                None
            };
            prepare_cli_open(
                prepared,
                values,
                clipboard.as_ref().and_then(|s| s.as_deref()),
            )
        }),
        mode,
        true,
        live,
        deadline,
        Box::new(move || grant()),
        dispatch,
        context,
    ) {
        Ok(_) => {
            state.open_validation_stamp = None;
            state.activity.record(&trace, &logged_id, "cli", None)
        }
        Err((error, mut context)) => {
            if let Some(span) = context.analytics.take() {
                span.finish(false, crate::analytics::failure_code(&error));
            }
            if let ExecutionDestination::Cli(request) = context.destination {
                let _ = request.respond(json!({"ok":false,"error":error}));
            }
        }
    }
}
#[allow(clippy::too_many_arguments)]
fn start_native_open(
    state: &mut SharedState,
    window: &mut WindowRuntime,
    ui: &Arc<Connection>,
    dismissal: u64,
    command: rustcast_core::Command,
    values: Value,
    input_token: Option<u64>,
) -> Result<(), String> {
    if state.execution.any(|c|matches!(&c.destination,ExecutionDestination::Native{ui:owner,..} if owner.ptr_eq(&Arc::downgrade(ui)))) {return Err("A command is already running in this window.".into());}
    let origin = if state
        .configuration
        .current()
        .commands
        .iter()
        .any(|c| execution_identity(&command, c))
    {
        ExecutionOrigin::Saved
    } else if state
        .applications
        .iter()
        .any(|c| execution_identity(&command, c))
        && crate::applications::available(&state.configuration.current().commands, &command)
    {
        ExecutionOrigin::Discovered(state.applications_revision)
    } else if service_file(&command) && state.system.command(&command.id).is_some_and(|c|execution_identity(&command,&c)) {
        ExecutionOrigin::Service(state.configuration.revision)
    } else {
        return Err("Selection changed. Try again.".into());
    };
    if command.kind == "app"
        && crate::applications::disabled_target(
            &state.configuration.current().commands,
            &command.target,
        )
    {
        return Err("Command is disabled.".into());
    }
    let services = state
        .services
        .clone()
        .ok_or("Main-thread service is unavailable.")?;
    let trace = uuid();
    let serial = window
        .execution_serial
        .checked_add(1)
        .ok_or("Execution sequence exhausted.")?;
    let key = crate::connected::HideKey {
        trace: trace.clone(),
        epoch: window.epoch,
        dismissal,
        intent: window.input_intent,
        serial,
    };
    let token = ui.arm_open_hide(key.clone())?;
    let weak = Arc::downgrade(ui);
    let check = key.clone();
    let live: crate::execution::Live = Arc::new(move || {
        weak.upgrade().is_some_and(|ui| {
            ui.open_hide_live(&check) && input_token.is_none_or(|t| !ui.editor_cancelled(t))
        })
    });
    // Like the shipped GUI, workspace opening has no script-derived wall timer.
    // Cancellation/identity/stop gates and finite capacities still apply.
    let deadline: Option<Instant> = native_open_deadline(state);
    let dispatch = Arc::new(crate::workspace::Dispatch::default());
    let permit = crate::main_services::Permit::default();
    let revoked = permit.live();
    let logged_id =
        crate::activity::Activity::log_id(analytics_id(&command), &values, &state.configuration.paths.home);
    let context = ExecutionContext {
        capture_result: None,
        initial_result: None,
        analytics: crate::analytics::Span::new(
            state.analytics.client(),
            analytics_id(&command),
            window.native_source,
        ),
        snippet_hide: None,
        destination: ExecutionDestination::Native {
            source: window.native_source,
            ui: Arc::downgrade(ui),
            epoch: window.epoch,
            dismissal,
            intent: window.input_intent,
            serial,
            input_token,
            result_origin: None,
        },
        command: command.clone(),
        file: state.configuration.file.clone(),
        trace: trace.clone(),
        logged_id: logged_id.clone(),
        log_secrets: log_secrets(&values),
        started: Instant::now(),
        origin,
        requested_key: None,
        _services: permit,
    };
    let grant = owner_grant(
        state.events.clone(),
        trace.clone(),
        live.clone(),
        deadline,
        false,
    );
    let hide_grant = owner_grant(
        state.events.clone(),
        trace.clone(),
        live.clone(),
        deadline,
        true,
    );
    let capture_grant = grant.clone();
    let capture_live = live.clone();
    let capture_services = services.clone();
    let capture_dispatch = dispatch.clone();
    let capture_revoked = revoked.clone();
    let prepare = Box::new(move || {
        let clipboard = if command.kind == "url"
            && crate::templates::needs_clipboard(&command.target)
        {
            capture_grant()?;
            capture_services.clipboard(deadline, capture_live, capture_revoked, capture_dispatch)?
        } else {
            None
        };
        prepare_cli_open(command, values, clipboard.as_deref())
    });
    let hide_live = live.clone();
    let hide_dispatch = dispatch.clone();
    let before = Box::new(move || {
        hide_grant()?;
        services.hide(token, deadline, hide_live, revoked, hide_dispatch)?;
        grant()
    });
    let mode = if state.fixture {
        #[cfg(feature = "test-tools")]
        {
            crate::execution::WorkspaceMode::Fixture(state.configuration.paths.home.clone())
        }
        #[cfg(not(feature = "test-tools"))]
        {
            unreachable!("Fixture requires test tools")
        }
    } else {
        crate::execution::WorkspaceMode::Production
    };
    match state.execution.admit_open_prepared_tracked(
        prepare, mode, false, live, deadline, before, dispatch, context,
    ) {
        Ok(_) => {
            window.execution_serial = serial;
            window.execution_effect = None;
            window.message.clear();
            window.session.actions_open = false;
            state.open_validation_stamp = None;
            state.activity.record(&trace, &logged_id, window.native_source, None);
            Ok(())
        }
        Err((error, mut context)) => {
            if let Some(span) = context.analytics.take() {
                span.finish(false, crate::analytics::failure_code(&error));
            }
            ui.cancel_open_hide();
            Err(error)
        }
    }
}
#[allow(clippy::too_many_arguments)]
fn start_execution(
    state: &mut SharedState,
    command: rustcast_core::Command,
    prepared: rustcast_core::Command,
    values: &Value,
    destination: ExecutionDestination,
    live: crate::execution::Live,
    deadline: Option<Instant>,
    initial_result: Option<u64>,
) -> Result<(), String> {
    let trace = uuid();
    let logged_id =
        crate::activity::Activity::log_id(&command.id, values, &state.configuration.paths.home);
    let source = destination.source();
    let context = ExecutionContext {
        capture_result: None,
        initial_result,
        analytics: crate::analytics::Span::new(
            state.analytics.client(),
            analytics_id(&command),
            source,
        ),
        snippet_hide: None,
        _services: Default::default(),
        destination,
        command,
        file: state.configuration.file.clone(),
        trace: trace.clone(),
        logged_id: logged_id.clone(),
        log_secrets: log_secrets(values),
        started: Instant::now(),
        origin: ExecutionOrigin::Saved,
        requested_key: None,
    };
    match state
        .execution
        .admit(prepared, source == "cli", live, deadline, context)
    {
        Ok(_) => {
            state.activity.record(&trace, &logged_id, source, None);
            Ok(())
        }
        Err((error, mut context)) => {
            if let Some(span) = context.analytics.take() {
                span.finish(false, crate::analytics::failure_code(&error));
            }
            if let ExecutionDestination::Cli(request) = context.destination {
                let _ = request.respond(json!({"ok":false,"error":error}));
            }
            Err(error)
        }
    }
}
fn admit_request(
    state: &mut SharedState,
    window: &mut WindowRuntime,
    ui: Option<&Arc<Connection>>,
    request: crate::control::Envelope,
) {
    // Management actions run on the serialized owner, never a script worker.
    if let Some(id)=request.value["id"].as_str() {
        if let Ok((command,_))=resolve_command(state,window,id) {
            if crate::management_commands::dashboard(&command) {
                let command=command.clone();
                let values=request.value.get("values").cloned().unwrap_or_else(||json!({}));
                let validation=if !request.context.is_live(){Err("Request expired or was cancelled.".into())}else{crate::management_commands::validate_input(&command,&values)};
                let run=validation.is_ok().then(||ManagementRun::begin(state,&command,"cli",&values));
                let result=validation.and_then(|()|if let Some(target)=ui.and_then(|u|u.peer()) {target.action(crate::connected::Action::OpenDashboard);Ok(())}else{Err("Settings window is unavailable.".into())});
                let value=if let Some(run)=run{run.finish(state,&command,"cli",&result)}else{match result {Ok(())=>json!({"ok":true}),Err(error)=>json!({"ok":false,"error":error})}};
                let _=request.respond(value);return;
            }
        }
    }
    if let Some(command)=request.value["id"].as_str().and_then(|id|resolve_command(state,window,id).ok()).map(|(c,origin)|(c.clone(),origin)).filter(|(c,_)|utilities::recognizes(c)) {
        let (command,origin)=command;
        let values=request.value.get("values").cloned().unwrap_or_else(||json!({}));
        if let Err(error)=crate::management_commands::validate_input(&command,&values){let _=request.respond(json!({"ok":false,"error":error}));return;}
        if utilities::kind(&command.target).is_some(){
            window.native_source="cli";
            let value=if let Some(ui)=ui {
                let (result,trace)=utilities::show_command(state,window,ui,&command,request.context.received_at(),&values);
                match result{Ok(())=>json!({"ok":true,"result":{"state":"opened"}}),Err(error)=>json!({"ok":false,"error":error,"diagnosticID":trace})}
            }else{let run=ManagementRun::begin(state,&command,"cli",&values);run.finish(state,&command,"cli",&Err("The palette is unavailable in headless mode.".into()))};
            let _=request.respond(value);return;
        }
        let context=request.context.clone();let deadline=Some(context.deadline());let requested_key=request.value["id"].as_str().map(command_key);
        let hide=ui.map(|ui|SnippetHideOrigin::capture(ui,None));let check=hide.clone();
        let revoked=window.utility.revoked.clone();let bound=ui.is_some();
        let live=Arc::new(move||context.is_live()&&(!bound||!revoked.load(std::sync::atomic::Ordering::Acquire))&&check.as_ref().is_none_or(SnippetHideOrigin::live));
        let _=utilities::start(state,command,values,ExecutionDestination::Cli(request),origin,requested_key,live,deadline,hide);return;
    }
    let prepare = || -> Result<_, String> {
        let id = request.value["id"].as_str().ok_or("Missing command id.")?;
        let (command, origin) = resolve_command(state, window, id)?;
        let values = request
            .value
            .get("values")
            .cloned()
            .unwrap_or_else(|| json!({}));
        if values
            .as_object()
            .is_none_or(|m| m.values().any(|v| !v.is_string()))
        {
            return Err("Input values must be strings.".into());
        }
        if ["app", "url", "path", "snippet"].contains(&command.kind.as_str()) {
            crate::execution::budget(&command, None)?;
            Ok((command.clone(), None, values, origin))
        } else {
            let prepared = crate::execution::prepare(&command, &values)?;
            Ok((command.clone(), Some(prepared), values, origin))
        }
    };
    match prepare() {
        Ok((command, None, values, origin)) => {
            if command.kind == "snippet" {
                let context = request.context.clone();
                let deadline = Some(context.deadline());
                let requested_key = request.value["id"].as_str().map(command_key);
                let hide = ui.map(|ui| SnippetHideOrigin::capture(ui, None));
                let check = hide.clone();
                let live: crate::execution::Live = Arc::new(move || {
                    context.is_live() && check.as_ref().is_none_or(SnippetHideOrigin::live)
                });
                let _ = start_snippet(
                    state,
                    command,
                    values,
                    ExecutionDestination::Cli(request),
                    origin,
                    requested_key,
                    live,
                    deadline,
                    hide,
                );
            } else {
                start_cli_open(state, command, origin, values, request);
            }
        }
        Ok((command, Some(prepared), values, _)) => {
            let context = request.context.clone();
            let deadline = context.deadline();
            let _ = start_execution(
                state,
                command,
                prepared,
                &values,
                ExecutionDestination::Cli(request),
                Arc::new(move || context.is_live()),
                Some(deadline),
                None,
            );
        }
        Err(error) => {
            let _ = request.respond(json!({"ok":false,"error":error}));
        }
    }
}
fn input_items(editing: &crate::actions::Editing) -> Vec<crate::actions::Item> {
    let enabled = editing
        .token()
        .and_then(|t| editing.input(t))
        .is_none_or(|(c, _, _)| c.enabled != Some(false));
    [
        ("input-run", "Run"),
        ("input-expand", "Expand Input…"),
        ("input-cancel", "Cancel Input"),
        ("edit", "Edit…"),
        ("set-alias", "Set Alias…"),
        ("set-hotkey", "Set Hotkey…"),
        (
            if enabled { "disable" } else { "enable" },
            if enabled { "Disable" } else { "Enable" },
        ),
        ("remove", "Remove…"),
        ("settings", "Settings"),
    ]
    .into_iter()
    .filter(|(id, _)| enabled || *id != "input-run")
    .map(|(id, title)| crate::actions::Item {
        id,
        title: title.into(),
        shortcut: None,
    })
    .collect()
}
fn capture_inline_query(state: &SharedState, window: &mut WindowRuntime, ui: &Connection) {
    if ui.is_management || !window.session.visible || window.editing.token().is_some() || state.execution.any(|c|matches!(&c.destination,ExecutionDestination::Native{ui:owner,..} if std::ptr::eq(owner.as_ptr(),ui))) { return; }
    let query = &window.session.query;
    let Some((index, ch)) = query.char_indices().find(|(_, c)| c.is_whitespace()) else {
        return;
    };
    let alias = command_key(&query[..index]);
    let value = &query[index + ch.len_utf8()..];
    let Some(command) = state.configuration.current().commands.iter().find(|c|name_matches(c,&alias)) else {
        return;
    };
    if matches!(command.kind.as_str(), "url" | "snippet") {
        let command = command.clone();
        let _ = request_fields(state, window, ui, &command, true);
        return;
    }
    if command.enabled == Some(false) || command.input_prompt.is_none()
        || (!utilities::recognizes(command) && !crate::system_search::apple_shortcuts::recognizes(command) && crate::execution::eligible(command).is_err())
        || crate::execution::budget(command, None).is_err()
    {
        return;
    }
    window
        .editing
        .open(&state.configuration, command, crate::actions::Mode::Input);
    let token = window.editing.token().expect("opened input");
    window
        .editing
        .update_input(token, 0, value)
        .expect("bounded input");
    window.editing.set_run_inline(token, true);
    window.editing.capture_input(token);
    let alias = alias.to_string();
    window.session.set_query(&alias);
    ui.adopt_query(&alias, window.epoch);
}
fn input_operation(
    state: &mut SharedState,
    window: &mut WindowRuntime,
    ui: &Arc<Connection>,
    token: u64,
    operation: crate::connected::InputOperation,
    dismissal: u64,
    attempt: u64,
) -> Result<(), String> {
    if state.execution.any(|c|matches!(&c.destination,ExecutionDestination::Native{ui:owner,..} if std::ptr::eq(owner.as_ptr(),Arc::as_ptr(ui)))) {return Err("A command is already running in this window.".into());}
    match operation {
        crate::connected::InputOperation::Run => {
            window.session.actions_open = false;
            submit_native_input(state, window, ui, token, dismissal)
        }
        crate::connected::InputOperation::Expand => {
            window.editing.set_run_inline(token, false);
            window.session.actions_open = false;
            Ok(())
        }
        crate::connected::InputOperation::Menu => {
            if !window.editing.run_inline() {
                return Err("This input is no longer inline.".into());
            }
            if ui.input_control_cancelled(attempt) {
                return Ok(());
            }
            window.editing.begin_input_menu(attempt);
            window.session.actions_open = true;
            Ok(())
        }
    }
}
fn expire_fields(window: &mut WindowRuntime, ui: &Connection) {
    if window
        .field_request
        .as_ref()
        .is_some_and(|r| Instant::now() >= r.deadline)
    {
        let request = window.field_request.take().expect("expired fields");
        window.field_preparation.cancel();
        if ui.latest_intent() == request.intent
            && ui.latest_epoch() == request.epoch
            && ui.latest_dismissal() == request.dismissal
            && window.session.visible
        {
            window.message = "Template preparation timed out. Try again.".into();
        }
    }
}
fn request_fields(
    state: &SharedState,
    window: &mut WindowRuntime,
    ui: &Connection,
    command: &rustcast_core::Command,
    alias: bool,
) -> Result<(), String> {
    if alias {
        window.native_source = "launcher";
    }
    if let Some(request) = window.field_request.as_mut().filter(|r| {
        r.command.id == command.id
            && r.config_revision == state.configuration.revision
            && r.alias == alias
    }) {
        request.source = window.native_source;
        request.epoch = window.epoch;
        request.intent = window.input_intent;
        request.dismissal = ui.latest_dismissal();
        request.launch = false;
        request.deadline = Instant::now() + Duration::from_secs(5);
        return Ok(());
    }
    let generation = window.field_preparation.request(command.clone())?;
    if alias {
        window.native_source = "launcher";
    }
    window.field_request = Some(FieldRequest {
        source: window.native_source,
        generation,
        command: command.clone(),
        file: state.configuration.file.clone(),
        config_revision: state.configuration.revision,
        epoch: window.epoch,
        intent: window.input_intent,
        dismissal: ui.latest_dismissal(),
        alias,
        launch: false,
        deadline: Instant::now() + Duration::from_secs(5),
    });
    if !alias {
        window.message = "Loading…".into();
    }
    Ok(())
}
fn finish_fields(state: &mut SharedState, window: &mut WindowRuntime, ui: &Arc<Connection>) {
    let Some((generation, result)) = window.field_preparation.take() else {
        return;
    };
    if window
        .field_request
        .as_ref()
        .is_none_or(|r| r.generation != generation)
    {
        return;
    }
    let request = window.field_request.take().expect("current field request");
    if window.message == "Loading…" {
        window.message.clear();
    }
    if Instant::now() >= request.deadline
        || ui.latest_epoch() != request.epoch
        || ui.latest_intent() != request.intent
        || window.epoch != request.epoch
        || ui.stop_requested()
        || ui.latest_dismissal() != request.dismissal
        || (!window.session.visible && request.source != "hotkey")
        || window.editing.token().is_some()
        || window.session.actions_open
    {
        return;
    }
    if state.reload_watched().is_err()
        || request.file != state.configuration.file
        || !state
            .configuration
            .current()
            .commands
            .iter()
            .any(|c| execution_identity(&request.command, c))
    {
        return;
    }
    window.input_intent = request.intent;
    let tail = if request.alias {
        let Some((at, ch)) = window
            .session
            .query
            .char_indices()
            .find(|(_, c)| c.is_whitespace())
        else {
            return;
        };
        let alias = &window.session.query[..at];
        let key = command_key(alias);
        let current = state
            .configuration
            .current()
            .commands
            .iter()
            .find(|c| name_matches(c, &key));
        if current.is_none_or(|c| !execution_identity(&request.command, c))
            || ui.latest_epoch() != window.epoch
        {
            return;
        }
        Some((
            alias.to_string(),
            window.session.query[at + ch.len_utf8()..].to_string(),
        ))
    } else {
        if ui.latest_epoch() != request.epoch
            || ui.latest_intent() != request.intent
            || window.epoch != request.epoch
        {
            return;
        }
        None
    };
    let fields = match result {
        Ok(fields) => fields,
        Err(error) => {
            if !request.alias {
                window.message = error;
            }
            return;
        }
    };
    if let Some((alias, tail)) = tail {
        if fields.0.len() != 1 || !fields.0[0].options.is_empty() {
            return;
        }
        window
            .editing
            .open_template(&state.configuration, &request.command, fields);
        let token = window.editing.token().expect("template opened");
        if let Err(error) = window.editing.update_input(token, 0, &tail) {
            window.editing.fail(error);
            return;
        }
        window.session.set_query(&alias);
        ui.adopt_query(&alias, window.epoch);
        if request.launch {
            if let Err(error) = submit_native_input(state, window, ui, token, request.dismissal) {
                window.editing.fail(error.clone());
                window.message = error;
            }
        }
    } else if fields.0.is_empty() {
        let command = request.command;
        let result = if command.kind == "snippet" {
            start_native_snippet(
                state,
                window,
                ui,
                request.dismissal,
                command,
                json!({}),
                None,
            )
        } else {
            start_native_open(
                state,
                window,
                ui,
                request.dismissal,
                command,
                json!({}),
                None,
            )
        };
        if let Err(error) = result {
            window.message = error;
        }
    } else {
        if request.source == "hotkey" && !window.session.visible {
            if window.show(Some(""), Instant::now(), ui).is_err() {
                return;
            }
            window.native_source = "hotkey";
        }
        let defaults: Vec<_> = fields
            .0
            .iter()
            .map(|f| (f.id.clone(), f.default_value.clone()))
            .collect();
        window
            .editing
            .open_template(&state.configuration, &request.command, fields);
        if request.launch {
            let token = window.editing.token().expect("template opened");
            if let Err(error) =
                submit_template(state, window, ui, token, request.dismissal, &defaults)
            {
                window.editing.fail(error.clone());
                window.message = error;
            }
        }
    }
}
fn native_open_deadline(state: &SharedState) -> Option<Instant> {
    #[cfg(feature = "test-tools")]
    if state.fixture {
        return std::env::var("RUYCAST_NATIVE_TEST_OPEN_DEADLINE_MS")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .filter(|v| *v > 0 && *v <= 5000)
            .map(|ms| Instant::now() + Duration::from_millis(ms));
    }
    let _ = state;
    None
}
fn admit_native(
    state: &mut SharedState,
    window: &mut WindowRuntime,
    ui: &Arc<Connection>,
    dismissal: u64,
) -> Result<(), String> {
    if ui.latest_dismissal() != dismissal || ui.stop_requested() {
        return Err("Request expired or was cancelled.".into());
    }
    let weak = Arc::downgrade(ui);
    if state
        .execution
        .any(|c| matches!(&c.destination,ExecutionDestination::Native{ui,..} if ui.ptr_eq(&weak)))
    {
        return Err("A command is already running in this window.".into());
    }
    window.native_source = "launcher";
    let selected = window.session.selected().ok_or("No command selected.")?;
    if utilities::recognizes(selected){let command=selected.clone();return admit_native_command(state,window,ui,dismissal,command);}
    if selected.kind=="builtin"&&selected.input_prompt.is_none()&&system::builtins::known(&selected.target){let command=selected.clone();return system::builtins::native(state,window,ui,&command,&json!({}),None);}
    if service_file(selected) && state.system.command(&selected.id).is_some_and(|c|execution_identity(selected,&c)){let command=selected.clone();return start_native_open(state,window,ui,dismissal,command,json!({}),None);}
    if (selected.kind=="window" && selected.input_prompt.is_none()) || system::ephemeral(selected) || crate::system_search::apple_shortcuts::recognizes(selected) {
        let command=selected.clone();return system::start_native(state,window,ui,&command);
    }
    if selected.kind=="builtin" && selected.input_prompt.is_none() {
        if let Some(scope)=system::builtin_scope(&selected.target) {
            window.system.enter(scope);window.session.set_query("");ui.adopt_query("",window.epoch);window.message.clear();if scope==crate::system_search::Scope::Tabs {system::consent(state,window,ui)?;}return Ok(());
        }
    }
    let command = crate::management_commands::catalog(&state.configuration.current().commands).into_iter().chain(state.applications.iter().cloned()).find(|c|execution_identity(selected,c)).ok_or("Selection changed. Try again.")?;
    admit_native_command(state, window, ui, dismissal, command)
}
fn admit_native_command(
    state: &mut SharedState,
    window: &mut WindowRuntime,
    ui: &Arc<Connection>,
    dismissal: u64,
    command: rustcast_core::Command,
) -> Result<(), String> {
    if command.enabled == Some(false) {
        return Err("Command is disabled.".into());
    }
    if utilities::recognizes(&command)&&utilities::kind(&command.target).is_some()&&command.input_prompt.is_none(){
        crate::management_commands::validate_input(&command,&json!({}))?;
        return utilities::show_command(state,window,ui,&command,Instant::now(),&json!({})).0;
    }
    if crate::management_commands::dashboard(&command) && command.input_prompt.is_none() {
        crate::management_commands::validate_input(&command,&json!({}))?;
        let run=ManagementRun::begin(state,&command,window.native_source,&json!({}));
        let target=if ui.is_management {Some(ui.clone())} else {ui.peer()};
        let result=target.ok_or_else(||"Settings window is unavailable.".to_string()).map(|target| {
            target.action(crate::connected::Action::OpenDashboard);
            if !ui.is_management {window.session.visible=false;window.session.actions_open=false;}
        });
        run.finish(state,&command,window.native_source,&result);
        return result;
    }
    if command.kind=="builtin"&&command.input_prompt.is_none()&&system::builtins::known(&command.target){return system::builtins::native(state,window,ui,&command,&json!({}),None);}
    if crate::system_search::apple_shortcuts::recognizes(&command) || system::ephemeral(&command) { return system::start_native(state,window,ui,&command); }
    if command.kind=="builtin" && command.input_prompt.is_none() { if let Some(scope)=system::builtin_scope(&command.target) { window.show(Some(""),Instant::now(),ui)?;window.system.enter(scope);if scope==crate::system_search::Scope::Tabs {system::consent(state,window,ui)?;}return Ok(()); } }
    let workspace = matches!(command.kind.as_str(), "app" | "url" | "path" | "snippet" | "window") || (command.kind=="builtin"&&system::builtins::known(&command.target));
    if !workspace && !crate::management_commands::dashboard(&command) && !utilities::recognizes(&command) {
        crate::execution::eligible(&command)?;
    }
    crate::execution::budget(&command, None)?;
    if matches!(command.kind.as_str(), "url" | "snippet") {
        let command = command.clone();
        return request_fields(state, window, ui, &command, false);
    }
    if command.input_prompt.is_some() {
        if window.native_source == "hotkey" && !window.session.visible {
            window.show(Some(""), Instant::now(), ui)?;
            window.native_source = "hotkey";
        }
        window
            .editing
            .open(&state.configuration, &command, crate::actions::Mode::Input);
        window
            .editing
            .set_run_inline(window.editing.token().expect("input opened"), true);
        window.editing.capture_input(window.editing.token().expect("input opened"));
        window.session.actions_open = false;
        window.message.clear();
        return Ok(());
    }
    let values = json!({});
    let prepared = if workspace || utilities::recognizes(&command) {
        command.clone()
    } else {
        crate::execution::prepare(&command, &values)?
    };
    let command = command.clone();
    begin_native_job(
        state, window, ui, dismissal, command, prepared, &values, None, None,
    )
}
#[allow(clippy::too_many_arguments)]
fn begin_native_job(
    state: &mut SharedState,
    window: &mut WindowRuntime,
    ui: &Arc<Connection>,
    dismissal: u64,
    command: rustcast_core::Command,
    prepared: rustcast_core::Command,
    values: &Value,
    input_token: Option<u64>,
    result_origin: Option<crate::script_session::Origin>,
) -> Result<(), String> {
    if ui.latest_dismissal() != dismissal
        || ui.stop_requested()
        || input_token.is_some_and(|t| ui.editor_cancelled(t))
    {
        return Err("Request expired or was cancelled.".into());
    }
    if crate::management_commands::dashboard(&command) {
        crate::management_commands::validate_input(&command,values)?;
        let run=ManagementRun::begin(state,&command,window.native_source,values);
        let target=if ui.is_management{Some(ui.clone())}else{ui.peer()};
        let result=target.ok_or_else(||"Settings window is unavailable.".to_string()).map(|target|{
            target.action(crate::connected::Action::OpenDashboard);
            if let Some(token)=input_token{window.editing.cancel(token);}
            if !ui.is_management{window.session.visible=false;window.session.actions_open=false;}
        });
        run.finish(state,&command,window.native_source,&result);return result;
    }
    if crate::system_search::apple_shortcuts::recognizes(&command) || system::ephemeral(&command) {
        return system::start_native_values(state,window,ui,&command,values,input_token);
    }
    if utilities::recognizes(&command){return utilities::start_native(state,window,ui,dismissal,command,values.clone(),input_token);}
    if command.kind=="builtin"&&system::builtins::known(&command.target){return system::builtins::native(state,window,ui,&command,values,input_token);}
    if command.kind=="window" {crate::builtin_effects::validate(&command,values)?;return system::start_native_values(state,window,ui,&command,values,input_token);}
    if command.kind == "snippet" {
        return start_native_snippet(
            state,
            window,
            ui,
            dismissal,
            command,
            values.clone(),
            input_token,
        );
    }
    if matches!(command.kind.as_str(), "app" | "url" | "path") {
        return start_native_open(
            state,
            window,
            ui,
            dismissal,
            command,
            values.clone(),
            input_token,
        );
    }
    let result_origin = result_origin.map(Arc::new);
    let weak = Arc::downgrade(ui);
    if state
        .execution
        .any(|c| matches!(&c.destination, ExecutionDestination::Native{ui,..} if ui.ptr_eq(&weak)))
    {
        return Err("A command is already running in this window.".into());
    }
    let serial = window.execution_serial.wrapping_add(1);
    let destination = ExecutionDestination::Native {
        source: window.native_source,
        ui: weak.clone(),
        epoch: window.epoch,
        dismissal,
        intent: window.input_intent,
        serial,
        input_token,
        result_origin: result_origin.clone(),
    };
    let initial_result = window.results.token().filter(|t| {
        window.results.command_id(*t) == Some(command.id.as_str())
    });
    start_execution(
        state,
        command,
        prepared,
        values,
        destination,
        Arc::new(move || {
            weak.upgrade().is_some_and(|ui| {
                !ui.stop_requested()
                    && ui.latest_dismissal() == dismissal
                    && input_token.is_none_or(|t| !ui.editor_cancelled(t))
                    && result_origin
                        .as_ref()
                        .is_none_or(|o| !ui.result_cancelled(o.token()))
            })
        }),
        None,
        initial_result,
    )?;
    window.execution_serial = serial;
    window.execution_effect = None;
    if window.native_source != "hotkey" {
        window.session.visible = true;
    }
    window.session.actions_open = false;
    window.message.clear();
    Ok(())
}
fn submit_native_input(
    state: &mut SharedState,
    window: &mut WindowRuntime,
    ui: &Arc<Connection>,
    token: u64,
    dismissal: u64,
) -> Result<(), String> {
    state.reload_watched()?;
    window.sync(state);
    let (base, input, file) = window
        .editing
        .input(token)
        .ok_or("This input is no longer open.")?;
    if file != state.configuration.file {
        return Err("Configuration folder changed. Reopen the command before running.".into());
    }
    let current = state
        .configuration
        .current()
        .commands
        .iter()
        .find(|c| execution_identity(base, c))
        .ok_or("Command changed. Reopen it before running.")?;
    crate::execution::budget(current, Some(input))?;
    let input = window.editing.input_execution_value(token)?;
    crate::execution::budget(current, Some(&input))?;
    let key = window
        .editing
        .view()
        .and_then(|e| e.command["inputKey"].as_str().map(str::to_string))
        .unwrap_or_else(|| "input".into());
    let values = json!({key:input});
    let prepared = if crate::management_commands::dashboard(current) || matches!(current.kind.as_str(), "app" | "url" | "path" | "snippet" | "window") || (current.kind=="builtin" && system::builtins::known(&current.target)) || crate::system_search::apple_shortcuts::recognizes(current) || utilities::recognizes(current) {
        current.clone()
    } else {
        crate::execution::prepare(current, &values)?
    };
    let command = current.clone();
    begin_native_job(
        state,
        window,
        ui,
        dismissal,
        command,
        prepared,
        &values,
        Some(token),
        None,
    )
}

fn submit_template(
    state: &mut SharedState,
    window: &mut WindowRuntime,
    ui: &Arc<Connection>,
    token: u64,
    dismissal: u64,
    supplied: &[(String, String)],
) -> Result<(), String> {
    state.reload_watched()?;
    window.sync(state);
    let view = window
        .editing
        .view()
        .filter(|e| e.token == token)
        .ok_or("This input is no longer open.")?;
    let fields = view
        .template_fields
        .as_ref()
        .ok_or("Template fields changed. Reopen the command.")?;
    let values = crate::template_form::values(fields, supplied)?;
    let (base, _, file) = window
        .editing
        .input(token)
        .ok_or("This input is no longer open.")?;
    if file != state.configuration.file {
        return Err("Configuration folder changed. Reopen the command before running.".into());
    }
    let command = state
        .configuration
        .current()
        .commands
        .iter()
        .find(|c| execution_identity(base, c))
        .ok_or("Command changed. Reopen it before running.")?
        .clone();
    begin_native_job(
        state,
        window,
        ui,
        dismissal,
        command.clone(),
        command,
        &values,
        Some(token),
        None,
    )
}
fn result_action(
    state: &mut SharedState,
    window: &mut WindowRuntime,
    ui: &Arc<Connection>,
    token: u64,
    name: &str,
    id: Option<&str>,
    dismissal: u64,
) -> Result<(), String> {
    if window.results.token() != Some(token) {
        return Ok(());
    }
    if name == "close" {
        window.results.back(token);
        window.message.clear();
        return Ok(());
    }
    match name {
        "details" => {
            window.results.show_detail(token, true);
        }
        "choices" => {
            window.results.show_detail(token, false);
        }
        "up" | "down" => {
            window.results.move_selection(token, if name == "up" { -1 } else { 1 });
            window.session.selection_scroll_serial = window.session.selection_scroll_serial.wrapping_add(1);
        }
        "select" => {
            if let Some(id) = id {
                window.results.select(token, id);
            }
        }
        "reload" | "activate" | "choose" => {
            let chosen = if name == "choose" {
                window.results.selected_id(token).map(str::to_owned)
            } else {
                id.map(str::to_owned)
            };
            let id = chosen.as_deref();
            state.reload_watched()?;
            window.sync(state);
            let command = state
                .configuration
                .current()
                .commands
                .iter()
                .find(|c| Some(c.id.as_str()) == window.results.command_id(token))
                .cloned()
                .ok_or("Command changed. Reopen it before running.")?;
            if name != "reload" && id.is_none() {
                return Err("No choice selected.".into());
            }
            if let Some(id) = id {
                if !window.results.select(token, id) {
                    return Err("Selection changed. Try again.".into());
                }
            }
            let file = state.configuration.file.clone();
            let request = window.results.prepare(
                token,
                if name == "reload" { None } else { id },
                &command,
                &file,
            )?;
            let mut owner = std::mem::take(&mut window.results);
            let admitted = owner.admit(request, &command, &file, |prepared, values, origin| {
                begin_native_job(
                    state,
                    window,
                    ui,
                    dismissal,
                    command.clone(),
                    prepared.clone(),
                    values,
                    None,
                    Some(origin.clone()),
                )
            });
            window.results = owner;
            admitted?;
        }
        _ => return Err("This result action is unavailable.".into()),
    }
    Ok(())
}
struct ManagementRun {
    trace: String,
    logged_id: String,
    secrets: Vec<String>,
    span: Option<crate::analytics::Span>,
    started: Instant,
}
impl ManagementRun {
    fn begin(state:&mut SharedState,command:&rustcast_core::Command,source:&str,values:&Value)->Self {
        let started=Instant::now();
        let trace=uuid();
        let span=crate::analytics::Span::new(state.analytics.client(),&command.id,source);
        let logged_id=crate::activity::Activity::log_id(&command.id,values,&state.configuration.paths.home);
        state.activity.record(&trace,&logged_id,source,None);
        Self {trace,logged_id,secrets:log_secrets(values),span,started}
    }
    fn finish(self,state:&mut SharedState,command:&rustcast_core::Command,source:&str,result:&Result<(),String>)->Value {
        if let Some(span)=self.span {span.finish(result.is_ok(),result.as_ref().err().map(|e|crate::analytics::failure_code(e)).unwrap_or(""));}
        let response=match result {Ok(())=>json!({"ok":true}),Err(error)=>json!({"ok":false,"error":error,"diagnosticID":self.trace})};
        state.activity.record_response(&self.trace,&self.logged_id,source,self.started.elapsed().as_secs_f64()*1000.,&response,(&self.secrets,&state.configuration.paths.home));
        if result.is_ok(){if let Err(error)=state.activity.success(&command.id){state.activity.storage_error=Some(error);}}
        response
    }
}
fn analytics_id(command: &rustcast_core::Command) -> &str {
    if service_file(command) {"system.file"} else if command.target.starts_with("system.tab:") {
        "system.tab"
    } else if command.target.starts_with("system.menu:") {
        "system.menu"
    } else if command.target.starts_with("system.window:") {
        "system.window"
    } else {
        &command.id
    }
}
fn finish_executions(
    state: &mut SharedState,
    palette: &mut WindowRuntime,
    management: &mut WindowRuntime,
    completed: Vec<crate::execution::Finished<ExecutionContext>>,
) -> Vec<mpsc::Receiver<bool>> {
    let mut deliveries = vec![];
    if completed.is_empty() {
        return deliveries;
    }
    // Filesystem observation is only a hint. Reconcile actual current storage
    // before attributing completed effects, including an event still queued.
    let current_configuration = state.reload_watched().is_ok();
    palette.sync(state);
    if management.session.library.is_some() {
        management.sync(state);
    }
    for finished in completed {
        let mut context = finished.context;
        let utility=utilities::recognizes(&context.command);
        let snippet = context.command.kind == "snippet" || utility;
        let snippet_live = context
            .snippet_hide
            .as_ref()
            .is_some_and(SnippetHideOrigin::live);
        if let Some(origin) = &context.snippet_hide {
            if let Some(token) = origin.slot.lock().unwrap_or_else(|p| p.into_inner()).take() {
                if let Some(ui) = origin.ui.upgrade() {
                    ui.finish_open_hide(&token.key);
                }
                if origin.native_serial.is_none()
                    && palette.epoch == origin.epoch
                    && palette.execution_serial == token.key.serial
                {
                    palette.execution_effect = None;
                }
            }
        }
        let (mut response, capture, mut script) = match finished.result {
            Ok(crate::execution::Outcome::Script(result)) => {
                (result.response, result.capture, true)
            }
            Ok(crate::execution::Outcome::Workspace(result)) => (result.response, None, false),
            Ok(crate::execution::Outcome::Snippet(result)) => (result.response, None, false),
            Err(error) => (json!({"ok":false,"error":error}), None, true),
        };
        let saved_capture = context.capture_result.take();
        if let Some(saved) = saved_capture.as_ref() {
            response = crate::script_result::capture_open_finished(saved.clone(), &response);
            script = true;
            if let ExecutionDestination::Native {
                ui, epoch, dismissal, intent, serial, ..
            } = &context.destination {
                if let Some(ui) = ui.upgrade() {
                    ui.finish_open_hide(&crate::connected::HideKey {
                        trace: context.trace.clone(), epoch: *epoch,
                        dismissal: *dismissal, intent: *intent, serial: *serial,
                    });
                }
            }
        }
        if matches!(context.origin,ExecutionOrigin::Clipboard(_))&&response["ok"]==true {response["result"]["state"]=json!("copied");}
        let audience = if matches!(&context.destination, ExecutionDestination::Cli(_)) {
            crate::script_result::Audience::Cli
        } else {
            crate::script_result::Audience::Native
        };
        let required = matches!(
            &context.destination,
            ExecutionDestination::Native {
                result_origin: Some(_),
                ..
            }
        );
        let mut interpreted = if script && saved_capture.is_none() {
            crate::script_result::interpret_command(
                &context.command, response, required, audience, capture.as_ref(),
            )
        } else {
            crate::script_result::Interpreted {
                response,
                choices: None,
                detail: None,
                envelope: false,
            }
        };
        if saved_capture.is_none()
            && !interpreted.envelope
            && matches!(context.destination, ExecutionDestination::Native { .. })
        {
            match crate::script_result::capture_open_request(&context.command, &interpreted.response) {
                Ok(Some(request)) => {
                    let window = if matches!(&context.destination, ExecutionDestination::Native { ui, .. } if ui.upgrade().is_some_and(|u| u.is_management)) {
                        &*management
                    } else {
                        &*palette
                    };
                    let saved = interpreted.response.clone();
                    match private::start_capture_open(state, window, context, saved.clone(), request) {
                        Ok(()) => continue,
                        Err(returned) => {
                            context = *returned;
                            context.capture_result = None;
                            interpreted.response = crate::script_result::capture_open_finished(
                                saved, &json!({"ok":false}),
                            );
                        }
                    }
                }
                Err(_) => {
                    interpreted.response = crate::script_result::capture_open_finished(
                        interpreted.response, &json!({"ok":false}),
                    );
                }
                Ok(None) => {}
            }
        }
        let mut response = interpreted.response.clone();
        if let Some(changed)=state.utility.changes.remove(&context.trace){
            crate::utility_storage::decorate_publication(&mut response,crate::utility_storage::retire_publication(&changed));
        }
        let success = response["ok"] == true;
        if let Some(span) = context.analytics.take() {
            span.finish_with_exit(
                success,
                crate::analytics::failure_code(response["error"].as_str().unwrap_or("")),
                response["exitCode"].as_i64(),
            );
        }
        let history_operation=matches!(context.origin,ExecutionOrigin::Clipboard(_));
        if !history_operation {state.activity.record_response(
            &context.trace,
            &context.logged_id,
            context.source(),
            context.started.elapsed().as_secs_f64() * 1000.,
            &response,
            (&context.log_secrets, &state.configuration.paths.home),
        );}
        if success && !history_operation && !matches!(context.origin, ExecutionOrigin::Service(_)) {
            if current_configuration && origin_valid(state, &context) {
                if context.source() == "launcher" {
                    if let Err(error) = state
                        .ranking
                        .record_success(&context.command.id, crate::activity::now())
                    {
                        response["rankingWarning"] = json!(error);
                    }
                }
                if let Err(error) = state.activity.success(&context.command.id) {
                    response["ok"] = json!(false);
                    response["error"] = json!(format!(
                        "Action completed, but usage could not be saved: {error}"
                    ));
                    response["actionCompleted"] = json!(true);
                }
            } else {
                response["usageWarning"] = json!(
                    "Command changed or its configuration is unavailable; usage was not attributed to a replacement."
                );
            }
        }
        if response["ok"] != true && !success {
            response["diagnosticID"] = json!(context.trace);
        }
        let current_origin = current_configuration && origin_valid(state, &context);
        match context.destination {
            ExecutionDestination::Cli(request) => {
                deliveries.push(request.respond(response));
            }
            ExecutionDestination::Native {
                source,
                ui,
                epoch,
                dismissal,
                intent,
                serial,
                input_token,
                result_origin,
            } => {
                let Some(ui) = ui.upgrade() else {
                    continue;
                };
                let window = if ui.is_management {
                    &mut *management
                } else {
                    &mut *palette
                };
                let warning_parts = [
                    (success && response["ok"] != true)
                        .then(|| response["error"].as_str())
                        .flatten(),
                    response["usageWarning"].as_str(),
                    response["rankingWarning"].as_str(),
                    response["durabilityWarning"].as_str(),
                ];
                let warning_text = warning_parts
                    .into_iter()
                    .flatten()
                    .collect::<Vec<_>>()
                    .join(
                        "
",
                    );
                let warning = (!warning_text.is_empty()).then_some(warning_text);
                if let Some(origin) = result_origin {
                    if !ui.stop_requested()
                        && ui.latest_dismissal() == dismissal
                        && window.execution_serial == serial
                        && !ui.result_cancelled(origin.token())
                    {
                        match window
                            .results
                            .complete(&origin, interpreted, warning.as_deref())
                        {
                            Ok(crate::script_session::Completion::Closed { notice }) => {
                                window.message = notice;
                                window.session.visible = !window.message.is_empty();
                            }
                            Ok(crate::script_session::Completion::Stale) => {}
                            Ok(_) => {
                                window.session.visible = true;
                            }
                            Err(error) => window.message = error,
                        }
                        if window.results.token() != Some(origin.token()) {
                            // Retire ingress authority before the fresh view is
                            // published, not after the main thread applies it.
                            ui.cancel_result(origin.token());
                        }
                    }
                    continue;
                }
                let known_completed = response["ok"] == true || response["actionCompleted"] == true;
                let bound_input = input_token.filter(|token| {
                    window.editing.token() == Some(*token)
                        && (snippet || window.execution_serial == serial)
                        && !ui.editor_cancelled(*token)
                });
                // Finalize the owned draft independently of optional palette effects.
                // A newer query/intent may prevent hiding, never offer this effect again.
                if let Some(token) = bound_input {
                    if known_completed {
                        window.editing.cancel(token);
                    } else {
                        window.editing.fail(
                            response["error"]
                                .as_str()
                                .unwrap_or("Script failed.")
                                .into(),
                        );
                    }
                }
                let hide_key = crate::connected::HideKey {
                    trace: context.trace.clone(),
                    epoch,
                    dismissal,
                    intent,
                    serial,
                };
                if (!script
                    && if snippet {
                        !snippet_live
                    } else {
                        !ui.open_hide_live(&hide_key)
                    })
                    || ui.stop_requested()
                    || ui.latest_dismissal() != dismissal
                    || ui.latest_epoch() != epoch
                    || ui.latest_intent() != intent
                    || window.execution_serial != serial
                    || window.session.actions_open
                    || window.epoch != epoch
                    || (if input_token.is_some() {
                        bound_input.is_none()
                    } else {
                        window.editing.token().is_some()
                    })
                    || input_token.is_some_and(|t| ui.editor_cancelled(t))
                    || (script
                        && source != "hotkey"
                        && input_token.is_none()
                        && (!current_origin
                            || window
                                .session
                                .selected()
                                .is_none_or(|c| c.id != context.command.id)))
                {
                    continue;
                }
                if snippet && known_completed {
                    let ok = response["ok"] == true && warning.is_none();
                    let text = if ok {
                        response["utilityNotice"].as_str().unwrap_or("Done").to_string()
                    } else {
                        response["error"].as_str().map(str::to_owned).or(warning).unwrap_or_else(||"The snippet may have changed the clipboard or posted input. Check its outcome before trying again.".into())
                    };
                    window.notify(text, ok);
                    window.message.clear();
                    // Preserve current visibility. A hidden palette must never
                    // reactivate over the user's now-active target application.
                    window
                        .execution_effect
                        .get_or_insert((intent, window.session.visible));
                    continue;
                }
                if response["ok"] == true && interpreted.envelope
                    && interpreted.choices.is_none() && interpreted.detail.is_none()
                {
                    if let Some(token) = context.initial_result {
                        if window.results.back(token) {
                            ui.cancel_result(token);
                        }
                    }
                }
                if interpreted.choices.is_some() || interpreted.detail.is_some() {
                    match window.results.present(
                        &context.command,
                        &context.file,
                        interpreted,
                        warning.as_deref(),
                    ) {
                        Ok(true) => {
                            window.session.visible = true;
                            window.message.clear();
                            window.execution_effect = None;
                            continue;
                        }
                        Ok(false) => {}
                        Err(error) => {
                            window.message = error;
                            continue;
                        }
                    }
                }
                if !script {
                    ui.finish_open_hide(&hide_key);
                    window
                        .execution_effect
                        .get_or_insert((intent, window.session.visible));
                    if known_completed {
                        if let Some(token) = bound_input {
                            window.editing.cancel(token);
                        }
                    }
                    if response["ok"] == true && warning.is_none() {
                        window.session.visible = snippet && ui.is_management;
                        window.message.clear();
                    } else {
                        window.session.visible = true;
                        let notice = if response["ok"] == true {
                            response["rankingWarning"]
                                .as_str()
                                .map(|error| {
                                    format!(
                                        "{}, but recent choices could not be saved: {error}",
                                        if snippet { "Pasted" } else { "Opened" }
                                    )
                                })
                                .or_else(|| {
                                    warning.map(|text| {
                                        format!(
                                            "{}. {text}",
                                            if snippet { "Pasted" } else { "Opened" }
                                        )
                                    })
                                })
                        } else {
                            warning
                        };
                        window.message = notice.unwrap_or_else(|| {
                            response["error"]
                                .as_str()
                                .unwrap_or(if snippet {
                                    "Snippet failed."
                                } else {
                                    "Opening failed."
                                })
                                .into()
                        });
                    }
                    continue;
                }
                window
                    .execution_effect
                    .get_or_insert((intent, window.session.visible));
                if let Some(token) = bound_input.filter(|_| known_completed) {
                    // Exact draft ownership survives unrelated catalog/usage refresh.
                    // Never leave a known completed effect offered as an unfinished Run.
                    window.editing.cancel(token);
                    let unchanged = current_configuration
                        && state
                            .configuration
                            .current()
                            .commands
                            .iter()
                            .any(|c| execution_identity(&context.command, c));
                    if response["ok"] == true && unchanged {
                        window.session.visible = false;
                        window.message.clear();
                    } else {
                        window.session.visible = true;
                        window.message = response["error"].as_str().unwrap_or("Action completed, but its configuration changed or is unavailable. Check its outcome before running again.").into();
                    }
                } else if response["ok"] == true {
                    window.session.visible = false;
                    window.message.clear();
                    if source == "hotkey" {
                        window.notify("Done".into(), true);
                    }
                } else {
                    window.message = response["error"]
                        .as_str()
                        .unwrap_or("Script failed.")
                        .into();
                    if source == "hotkey" {
                        window.notify(window.message.clone(), false);
                    }
                }
            }
        }
    }
    deliveries
}

fn creation_live(
    r: &CreationRequest,
    state: &SharedState,
    window: &WindowRuntime,
    ui: &Connection,
) -> bool {
    window.editing.token() == Some(r.token)
        && !state.config_busy()
        && !state.config_dirty
        && window.epoch == r.epoch
        && window.session.visible
        && state.configuration.file == r.file
        && state.configuration.revision == r.revision
        && state.configuration.last_error.is_none()
        && ui.latest_epoch() == r.epoch
        && ui.latest_intent() == r.intent
        && ui.latest_dismissal() == r.dismissal
        && !ui.stopped()
        && !ui.stop_requested()
        && !ui.editor_cancelled(r.token)
        && !window.editing.cancelled(ui)
        && !r.menu.is_some_and(|m| ui.menu_cancelled(m))
        && Instant::now() < r.deadline
}
fn finish_creation(state: &mut SharedState, window: &mut WindowRuntime, ui: &Arc<Connection>) {
    let Some(r) = window.creation_request.as_ref() else {
        return;
    };
    if !creation_live(r, state, window, ui) {
        let token = r.token;
        window.creation_request = None;
        window.editing.cancel(token);
        window.stamp = None;
        return;
    }
    if !r.requested && r.fixture_at.is_none() {
        // Bind after InlineGlobal has attached its actual originating editor.
        let parent = window.editing.view().and_then(|v| v.origin_inline);
        let (epoch, intent, dismissal, token, menu, deadline) =
            (r.epoch, r.intent, r.dismissal, r.token, r.menu, r.deadline);
        let weak = Arc::downgrade(ui);
        let live: crate::execution::Live = Arc::new(move || {
            weak.upgrade().is_some_and(|ui| {
                !ui.stopped()
                    && !ui.stop_requested()
                    && ui.latest_epoch() == epoch
                    && ui.latest_intent() == intent
                    && ui.latest_dismissal() == dismissal
                    && !ui.editor_cancelled(token)
                    && !parent.is_some_and(|p| ui.editor_cancelled(p))
                    && !menu.is_some_and(|m| ui.menu_cancelled(m))
            })
        });
        let reply = state.services.as_ref().and_then(|services| {
            let events = state.events.clone();
            services
                .begin_prefill(
                    deadline,
                    live,
                    r._permit.live(),
                    Arc::new(move || {
                        let _ = events.try_send(HostEvent::UiWake);
                    }),
                )
                .ok()
        });
        let r = window.creation_request.as_mut().unwrap();
        r.requested = true;
        r.reply = reply;
        if r.reply.is_some() {
            return;
        }
    }
    let r = window.creation_request.as_ref().unwrap();
    let text = if let Some(at) = r.fixture_at {
        if Instant::now() < at {
            return;
        };
        Some("https://example.com/delayed-prefill".to_string())
    } else if let Some(reply) = &r.reply {
        let Some(result) = reply.take() else { return };
        result.ok().flatten()
    } else {
        None
    };
    // A ready reply has no authority until the actual current file is reloaded.
    let fresh = state.reload_watched().is_ok();
    let r = window.creation_request.take().expect("creation request");
    window.stamp = None;
    if !fresh || !creation_live(&r, state, window, ui) {
        window.editing.cancel(r.token);
        return;
    }
    if let Some(text) = text {
        window.editing.prefill_created(r.token, text);
    }
}
fn start_creation(
    state: &SharedState,
    window: &mut WindowRuntime,
    ui: &Arc<Connection>,
    menu: Option<u64>,
    dismissal: u64,
    deadline: Instant,
) {
    let token = window.editing.token().expect("reserved new draft");
    let permit = crate::main_services::Permit::default();
    let epoch = window.epoch;
    let intent = ui.latest_intent();
    let fixture_at = ui
        .take_test_prefill()
        .then(|| Instant::now() + Duration::from_millis(500));
    window.creation_request = Some(CreationRequest {
        token,
        file: state.configuration.file.clone(),
        revision: state.configuration.revision,
        epoch,
        intent,
        dismissal,
        menu,
        deadline,
        _permit: permit,
        reply: None,
        fixture_at,
        requested: false,
    });
    window.stamp = None;
}

fn process_inputs(
    state: &mut SharedState,
    window: &mut WindowRuntime,
    ui: &Arc<Connection>,
) -> bool {
    if ui.stop_requested() {
        return true;
    }
    // Revoke an old creation before processing a replacement action/query.
    if window
        .creation_request
        .as_ref()
        .is_some_and(|r| !creation_live(r, state, window, ui))
    {
        let token = window.creation_request.take().unwrap().token;
        window.editing.cancel(token);
        window.stamp = None;
    }
    if let Some((intent, visible)) = window.execution_effect {
        if ui.latest_intent() != intent {
            window.execution_effect = None;
            window.session.visible = visible;
            window.message.clear();
        }
    }
    let cancelled_active = if window.editing.cancelled(ui) {
        let token = window.editing.token().expect("cancelled editor");
        window.editing.cancel(token);
        Some(token)
    } else {
        None
    };
    if ui.menu_cancelled(window.editing.menu_token) {
        window.session.actions_open = false;
    }
    if window
        .editing
        .input_menu_attempt()
        .is_some_and(|a| ui.input_control_cancelled(a))
    {
        window.session.actions_open = false;
    }
    if let Some(t) = window.results.token().filter(|t| ui.result_cancelled(*t)) {
        window.results.back(t);
        window.message.clear();
    }
    let (query, mut actions, overflow) = ui.drain();
    if let Some(input) = window.deferred_save.take() {
        let token = match &input.action {
            Action::SaveEditor { token, .. } => *token,
            _ => unreachable!("only editor saves are deferred"),
        };
        if window.editing.token() != Some(token)
            || input.dismissal != ui.latest_dismissal()
            || !window.session.visible
        {
            // Cancellation/replacement has already retired the old draft.
            if window.editing.token() == Some(token) {
                window.editing.set_saving(false);
            }
        } else if Instant::now() >= input.deadline {
            window.editing.set_saving(false);
            window
                .editing
                .fail("Save request expired. Try again.".into());
        } else if state.config_busy() || state.config_dirty {
            window.deferred_save = Some(input);
        } else {
            window.editing.set_saving(false);
            actions.insert(0, input);
        }
    }
    if let Some((next, query, received, marked, result_token)) = query {
        window.input_intent = ui.latest_intent();
        window.query_intent = Some(received);
        window.epoch = next;
        if let Some(token) = result_token {
            if window.results.token() == Some(token) && !ui.result_cancelled(token) {
                if let Err(error) = window.results.query(token, &query) {
                    window.message = error;
                }
            }
        } else if window.results.token().is_none() {
            window.session.set_query(&query);
            if !marked && !window.session.clipboard_mode {
                capture_inline_query(state, window, ui);
            }
            window.message.clear();
        }
    }
    if overflow {
        window.message = "Busy. Try again.".into();
    }
    for input in actions {
        // A native request can already be queued when its result is closed or
        // replaced. Retired tokens have no UI/message/intent authority.
        if matches!(&input.action, Action::Result { token, .. } if window.results.token() != Some(*token))
        {
            continue;
        }
        window.input_intent = input.intent;
        if let Some(request) = window.field_request.as_mut().filter(|request| {
            input.intent >= request.intent
                && input.epoch >= request.epoch
                && Instant::now() < input.deadline
        }) {
            if matches!(&input.action,Action::Key(key) if key=="insertNewline:")
                && input.dismissal == request.dismissal
                && input.epoch == request.epoch
                && Instant::now() < input.deadline
            {
                request.launch = true;
                request.intent = input.intent;
                continue;
            }
            window.field_request = None;
            window.field_preparation.cancel();
            if window.message == "Loading…" {
                window.message.clear();
            }
        }
        if let Action::Result {
            token,
            name,
            id,
            epoch: view_epoch,
        } = &input.action
        {
            if input.dismissal == ui.latest_dismissal()
                && *view_epoch == window.epoch
                && input.epoch == window.epoch
                && Instant::now() < input.deadline
                && window.session.visible
            {
                if let Err(error) = result_action(
                    state,
                    window,
                    ui,
                    *token,
                    name,
                    id.as_deref(),
                    input.dismissal,
                ) {
                    window.results.fail(*token, &error);
                    window.message = error;
                }
            }
            continue;
        }
        if let Action::Monitor{token,operation}=input.action {
            if !ui.stop_requested()&&Instant::now()<input.deadline{system::monitor_operation(state,token,operation);}
            continue;
        }
        // Keep dismissal usable while a single save owns this draft. New
        // mutations must not change its completion identity or repeat a write.
        if window.editing.is_saving()
            && !matches!(
                &input.action,
                Action::CancelEditor { .. } | Action::CloseActions { .. } | Action::Show
            )
            && !matches!(&input.action, Action::Key(key) if key == "cancelOperation:")
        {
            continue;
        }
        let previous = match &input.action {
            Action::BeginInline { previous, .. }
            | Action::SetEnabled { previous, .. }
            | Action::Library { previous, .. }
            | Action::InlineGlobal { previous, .. } => previous.as_ref(),
            _ => None,
        };
        let inline_ack = previous
            .is_some_and(|p| !window.editing.cancelled(ui) && window.editing.acknowledge_inline(p));
        if let Action::Setting {revision,navigation_revision,operation}=&input.action {
            let valid=ui.is_management && window.session.visible && window.editing.token().is_none()
                && window.session.library.as_ref().is_some_and(|n|["general","advanced","clipboard"].contains(&n.tab.as_str()))
                && *revision==state.configuration.revision && *navigation_revision==window.session.navigation_revision && input.epoch==window.epoch
                && input.dismissal==ui.latest_dismissal() && Instant::now()<input.deadline;
            if valid {
                use crate::settings_surface::Operation;
                let operation=operation.clone();
                let result=match operation {
                    Operation::ClipboardPause(paused)=>{
                        if window.session.library.as_ref().is_some_and(|n|n.tab=="clipboard") {
                            match state.clipboard.as_mut() {
                                Some(store)=>store.pause(paused,||!ui.stop_requested()&&ui.latest_epoch()==input.epoch&&ui.latest_dismissal()==input.dismissal&&Instant::now()<input.deadline).map(|receipt|{window.message=receipt.warning.unwrap_or_default();window.stamp=None;}),
                                None=>Err("Clipboard history is unavailable.".into()),
                            }
                        }else{Err("Clipboard settings are unavailable.".into())}
                    },
                    Operation::Set{key,value}=>start_configuration_edit(state,window,ui,input.deadline,move |store,_,_,_|Ok((crate::settings_surface::update(store,&key,value)?,ConfigAfter::None))),
                    Operation::Reload=>start_configuration_edit(state,window,ui,input.deadline,move |store,_,_,_|{store.reload()?;Ok((store.last_error.clone(),ConfigAfter::None))}),
                    Operation::Directory(directory)=>start_configuration_edit(state,window,ui,input.deadline,move |store,_,_,_|Ok((store.relocate(&directory)?.warning,ConfigAfter::None))),
                    Operation::Feedback(message)=>{window.message=message;Ok(())}
                };
                if let Err(error)=result{window.message=error;}
            }
            continue;
        }
        if let Action::Dashboard { token, operation } = &input.action {
            let valid = ui.is_management && window.session.visible
                && window.session.library.as_ref().is_some_and(|n|n.tab=="dashboard")
                && *token == window.dashboard.token() && input.epoch==window.epoch
                && input.dismissal==ui.latest_dismissal() && Instant::now()<input.deadline;
            if valid {
                let result=window.dashboard.operation(&state.analytics.client(),operation,state.diagnostics.busy());
                let result=result.and_then(|()|if let crate::analytics_dashboard::Operation::Export(path)=operation {
                    state.diagnostics.begin(&state.analytics.client(),path.clone(),*token,input.dismissal,state.events.clone())
                } else {Ok(())});
                if let Err(error)=result { window.dashboard.fail(error); }
            }
            continue;
        }
        if matches!(input.action, Action::Show | Action::OpenDashboard) {
            if Instant::now() < input.deadline
                && input.dismissal == ui.latest_dismissal()
                && window
                    .visibility_intent
                    .is_none_or(|last| input.received >= last)
            {
                window.visibility_intent = Some(input.received);
                if matches!(input.action, Action::Show) && !ui.is_management {
                    if window.session.visible && window.results.token().is_none()
                        && !window.session.clipboard_mode && window.session.launcher_scope.is_none()
                        && window.utility.mode.is_none() && window.session.library.is_none()
                        && window.system.scope == crate::system_search::Scope::MainTabs
                    {
                        window.retire_creation();
                        window.field_preparation.cancel();
                        window.field_request = None;
                        window.editing.cancel_all();
                        window.session.actions_open = false;
                        window.session.visible = false;
                        ui.cancel_open_hide();
                    } else if let Err(error) = window.show(Some(""), input.received, ui) {
                        window.message = error;
                    } else {
                        let _ = system::consent(state, window, ui);
                    }
                } else {
                    if !ui.is_management { window.system.enter(crate::system_search::Scope::MainTabs); }
                    if matches!(input.action,Action::OpenDashboard) && ui.is_management {
                        let serial=window.dashboard_request.map(|v|v.0).unwrap_or(0).checked_add(1);
                        window.dashboard_request=serial.map(|s|(s,input.intent,input.dismissal));
                    }
                    window.session.visible = true;
                    window.show_serial = window.show_serial.wrapping_add(1);
                }
            }
            continue;
        }
        if let Action::Library {
            kind,
            value,
            catalog_revision,
            previous,
        } = &input.action
        {
            let valid = window.session.library.is_some()
                && window.session.visible
                && input.dismissal == ui.latest_dismissal()
                && Instant::now() < input.deadline
                && (kind != "group"
                    || (*catalog_revision == window.session.results_revision()
                        && input.epoch == window.epoch
                        && (window.editing.token().is_none() || previous.is_some())));
            if !valid {
                if inline_ack {
                    window.editing.fail("Selection changed. Try again.".into());
                }
                continue;
            }
            if kind=="tab" && window.session.library.as_ref().is_some_and(|n|n.tab!=*value) {
                window.dashboard.retire();
            }
            if previous.is_some() {
                let previous = previous.clone();
                let kind = kind.clone();
                let value = value.clone();
                if let Err(error) = start_configuration_edit(
                    state,
                    window,
                    ui,
                    input.deadline,
                    move |store, editing, _, _| {
                        let warning = editing.commit_inline(store, previous)?;
                        Ok((warning, ConfigAfter::Navigate { kind, value }))
                    },
                ) {
                    window.editing.fail(error.clone());
                    window.message = error;
                }
            } else {
                window.editing.cancel_all();
                window.session.actions_open = false;
                match window.session.navigate(kind, value) {
                    Ok(()) => ui.adopt_query(&window.session.query, window.epoch),
                    Err(error) => window.message = error,
                }
            }
            continue;
        }
        let closing = matches!(&input.action,Action::Key(key) if key=="cancelOperation:");
        if closing {
            window.dismissal = window.dismissal.max(input.dismissal);
            if window
                .visibility_intent
                .is_none_or(|last| input.received >= last)
            {
                window.visibility_intent = Some(input.received);
                window.session.visible = false;
                window.session.actions_open = false;
                window.editing.cancel_all();
            }
            continue;
        }
        if window.session.library.as_ref().is_some_and(|n|["dashboard","general","advanced"].contains(&n.tab.as_str())) {continue;}
        match &input.action {
            Action::CancelEditor { token } => {
                let current =
                    window.editing.token() == Some(*token) || cancelled_active == Some(*token);
                window.editing.cancel(*token);
                if current {
                    window.message.clear();
                }
                continue;
            }
            Action::CloseActions { token } => {
                if window.editing.menu_token == *token {
                    window.session.actions_open = false;
                }
                continue;
            }
            _ => {}
        }
        if let Action::SubmitTemplate {
            token,
            attempt,
            values,
        } = &input.action
        {
            if window.editing.input(*token).is_none() || window.editing.cancelled(ui) {
                continue;
            }
            let result = window
                .editing
                .update_input(*token, *attempt, "")
                .and_then(|_| {
                    if Instant::now() >= input.deadline
                        || input.epoch != window.epoch
                        || input.dismissal != ui.latest_dismissal()
                        || !window.session.visible
                    {
                        Err("Run request expired. Try again.".into())
                    } else {
                        submit_template(state, window, ui, *token, input.dismissal, values)
                    }
                });
            if let Err(error) = result {
                window.editing.fail(error.clone());
                window.message = error;
            }
            continue;
        }
        if let Action::PreviewInput { token, value } = &input.action {
            if input.epoch == window.epoch
                && input.dismissal == ui.latest_dismissal()
                && Instant::now() < input.deadline
                && window.session.visible
                && !window.editing.cancelled(ui)
                && !state.execution.any(|c|matches!(&c.destination,ExecutionDestination::Native{ui:owner,..} if std::ptr::eq(owner.as_ptr(),Arc::as_ptr(ui))))
            {
                if let Some(editor) = window.editing.view().filter(|e| e.token == *token && e.task_date.is_some()) {
                    match window.editing.update_input(*token, editor.attempt, value) {
                        Ok(()) => window.message.clear(),
                        Err(error) => window.editing.fail(error),
                    }
                }
            }
            continue;
        }
        let submission = match &input.action {
            Action::SubmitInput {
                token,
                attempt,
                value,
            } => Some((
                *token,
                *attempt,
                value,
                crate::connected::InputOperation::Run,
            )),
            Action::InputControl {
                token,
                attempt,
                value,
                operation,
            } => Some((*token, *attempt, value, *operation)),
            _ => None,
        };
        if let Some((token, attempt, value, operation)) = submission {
            if window.editing.input(token).is_none() || window.editing.cancelled(ui) {
                continue;
            }
            let update = window.editing.update_input(token, attempt, value);
            let result = if let Err(error) = update {
                Err(error)
            } else if Instant::now() >= input.deadline
                || input.epoch != window.epoch
                || input.dismissal != ui.latest_dismissal()
                || !window.session.visible
            {
                Err("Run request expired. Try again.".into())
            } else {
                input_operation(
                    state,
                    window,
                    ui,
                    token,
                    operation,
                    input.dismissal,
                    attempt,
                )
            };
            if let Err(error) = result {
                window.editing.fail(error.clone());
                window.message = error;
            }
            continue;
        }
        if let Action::InputMenu {
            token,
            menu_token,
            key,
            name,
        } = &input.action
        {
            if !window.editing.run_inline()
                || window.editing.token() != Some(*token)
                || window.editing.menu_token != *menu_token
                || !window.session.actions_open
                || ui.menu_cancelled(*menu_token)
                || ui.editor_cancelled(*token)
                || window
                    .editing
                    .input_menu_attempt()
                    .is_some_and(|a| ui.input_control_cancelled(a))
                || input.dismissal != ui.latest_dismissal()
                || input.epoch != window.epoch
                || Instant::now() >= input.deadline
            {
                continue;
            }
            let items = input_items(&window.editing);
            let chosen = if let Some(name) = name {
                Some(name.clone())
            } else {
                match key.as_deref() {
                    Some("moveDown:" | "insertTab:") => {
                        window.editing.action_selection =
                            (window.editing.action_selection + 1).min(items.len() - 1);
                        None
                    }
                    Some("moveUp:" | "insertBacktab:") => {
                        window.editing.action_selection =
                            window.editing.action_selection.saturating_sub(1);
                        None
                    }
                    Some("insertNewline:") => items
                        .get(window.editing.action_selection)
                        .map(|i| i.id.into()),
                    _ => None,
                }
            };
            if let Some(chosen) = chosen {
                window.session.actions_open = false;
                let result = match chosen.as_str() {
                    "input-run" => input_operation(
                        state,
                        window,
                        ui,
                        *token,
                        crate::connected::InputOperation::Run,
                        input.dismissal,
                        0,
                    ),
                    "input-expand" => input_operation(
                        state,
                        window,
                        ui,
                        *token,
                        crate::connected::InputOperation::Expand,
                        input.dismissal,
                        0,
                    ),
                    "input-cancel" => {
                        ui.action(Action::CancelEditor { token: *token });
                        window.editing.cancel(*token);
                        window.session.actions_open = false;
                        Ok(())
                    }
                    "edit" | "set-alias" | "set-hotkey" | "remove" => {
                        let mode = match chosen.as_str() {
                            "edit" => crate::actions::Mode::Edit,
                            "set-alias" => crate::actions::Mode::Alias,
                            "set-hotkey" => crate::actions::Mode::Hotkey,
                            _ => crate::actions::Mode::Remove,
                        };
                        let menu = *menu_token;
                        start_configuration_edit(
                            state,
                            window,
                            ui,
                            input.deadline,
                            move |store, editing, _, _| {
                                editing.edit_input(store, mode)?;
                                editing.bind_origin_menu(Some(menu));
                                Ok((None, ConfigAfter::None))
                            },
                        )
                    }
                    "disable" | "enable" => {
                        let enabled = chosen == "enable";
                        start_configuration_edit(
                            state,
                            window,
                            ui,
                            input.deadline,
                            move |store, editing, _, _| {
                                let receipt = editing.enable_input(store, enabled)?;
                                Ok((saved_warning(receipt), ConfigAfter::None))
                            },
                        )
                    }
                    "settings" => {
                        if let Some(peer) = ui.peer() {
                            peer.action(Action::Show);
                            window.session.actions_open = false;
                            Ok(())
                        } else {
                            Err("Settings window is unavailable.".into())
                        }
                    }
                    _ => Err("This input action is unavailable.".into()),
                };
                if let Err(error) = result {
                    window.editing.fail(error.clone());
                    window.message = error;
                }
            }
            continue;
        }
        if let Action::SaveEditor {
            token,
            attempt,
            aliases,
            hotkey,
            command,
        } = &input.action
        {
            if window.editing.token() != Some(*token) || window.editing.cancelled(ui) {
                continue;
            }
            window
                .editing
                .update_draft(*token, *attempt, aliases.clone(), hotkey.clone());
            if let Some(command) = command {
                window.editing.update_command(*token, command.clone());
            }
            if Instant::now() >= input.deadline {
                window
                    .editing
                    .fail("Save request expired. Try again.".into());
                continue;
            }
        }
        if (input.epoch != window.epoch && !matches!(&input.action, Action::SaveEditor { .. }))
            || Instant::now() >= input.deadline
            || !window.session.visible
        {
            if inline_ack {
                window
                    .editing
                    .fail("This change expired. Try again.".into());
            }
            continue;
        }
        if matches!(&input.action, Action::SaveEditor { .. })
            && state.config_busy()
            && matches!(state.config_context, Some(ConfigContext::Watch))
        {
            // A coalesced notification from the previous save can start a
            // reload after its ACK. Do not reject the user's next save for
            // that internal work. Draft Saving suppresses duplicate mutations;
            // cancel/hide and the original expiry remain effective while held.
            window.editing.set_saving(true);
            window.deferred_save = Some(input);
            continue;
        }
        if window.session.clipboard_mode && clipboard_input(state,window,ui,&input) {continue;}
        if let Action::LauncherBack(scope) = &input.action {
            if window.session.launcher_scope == Some(*scope) && window.editing.token().is_none() && window.results.token().is_none() {
                if let Err(error) = window.show(Some(""), input.received, ui) { window.message = error; }
                window.stamp = None;
            }
            continue;
        }
        match input.action {
            Action::LauncherBack(_) => {},
            Action::ProcessSort(sort)=>{if window.system.scope==crate::system_search::Scope::Processes && window.editing.token().is_none(){system::process_sort(window,sort);}},
            Action::InlineGlobal { name, previous } => {
                if !ui.is_management
                    || !["new-snippet", "new-quicklink", "new-script"].contains(&name.as_str())
                    || window.editing.cancelled(ui)
                    || input.dismissal != ui.latest_dismissal()
                {
                    if inline_ack {
                        window
                            .editing
                            .fail("This action is unavailable. Try again.".into());
                    }
                    continue;
                }
                let origin = previous.as_ref().map(|p| p.token);
                if let Err(error) = start_configuration_edit(
                    state,
                    window,
                    ui,
                    input.deadline,
                    move |store, editing, _, _| {
                        let warning = editing.commit_inline(store, previous)?;
                        Ok((warning, ConfigAfter::Named { name, origin }))
                    },
                ) {
                    window.editing.fail(error.clone());
                    window.message = error;
                }
            }
            Action::BeginInline {
                target,
                mode,
                previous,
            } => {
                if !ui.is_management
                    || input.dismissal != ui.latest_dismissal()
                    || target.epoch != window.epoch
                    || target.catalog_revision != window.session.results_revision()
                    || previous
                        .as_ref()
                        .is_some_and(|p| ui.editor_cancelled(p.token))
                {
                    if inline_ack {
                        window.editing.fail("Selection changed. Try again.".into());
                    }
                    continue;
                }
                let command = window.session.rows().find(|c| c.id == target.id).cloned();
                if let Some(command) = command {
                    if let Err(error) = start_configuration_edit(
                        state,
                        window,
                        ui,
                        input.deadline,
                        move |store, editing, _, live| {
                            let outcome = editing.transition_inline(
                                store,
                                &command,
                                mode,
                                previous,
                                || live(),
                            )?;
                            if let Some(error) = outcome.error {
                                // The previous inline save may already be published.
                                editing.fail(error.clone());
                                return Ok((
                                    Some(
                                        [outcome.warning.unwrap_or_default(), error]
                                            .into_iter()
                                            .filter(|s| !s.is_empty())
                                            .collect::<Vec<_>>()
                                            .join("\n"),
                                    ),
                                    ConfigAfter::None,
                                ));
                            }
                            Ok((
                                outcome.warning,
                                if outcome.opened {
                                    ConfigAfter::Select(command.id)
                                } else {
                                    ConfigAfter::None
                                },
                            ))
                        },
                    ) {
                        window.editing.fail(error.clone());
                        window.message = error;
                    }
                } else if inline_ack {
                    window.editing.fail("Command changed. Try again.".into());
                }
            }
            Action::SetEnabled {
                target,
                expected,
                enabled,
                previous,
            } => {
                if !ui.is_management
                    || window.session.actions_open
                    || previous
                        .as_ref()
                        .is_some_and(|p| ui.editor_cancelled(p.token))
                    || input.dismissal != ui.latest_dismissal()
                    || target.epoch != window.epoch
                    || target.catalog_revision != window.session.results_revision()
                {
                    if inline_ack {
                        window.editing.fail("Selection changed. Try again.".into());
                    }
                    continue;
                }
                let command = window.session.rows().find(|c| c.id == target.id).cloned();
                if let Some(command) = command.filter(|c| (c.enabled != Some(false)) == expected) {
                    if let Err(error) = start_configuration_edit(
                        state,
                        window,
                        ui,
                        input.deadline,
                        move |store, editing, apps, live| {
                            let first = editing.commit_inline(store, previous)?;
                            if !live() {
                                return Ok((first, ConfigAfter::None));
                            }
                            let receipt = crate::actions::set_enabled_with_apps(
                                store, &command, enabled, apps,
                            )
                            .map_err(|error| {
                                [first.clone().unwrap_or_default(), error]
                                    .into_iter()
                                    .filter(|s| !s.is_empty())
                                    .collect::<Vec<_>>()
                                    .join("\n")
                            })?;
                            let warning = [
                                first.unwrap_or_default(),
                                receipt.warning.unwrap_or_default(),
                            ]
                            .into_iter()
                            .filter(|s| !s.is_empty())
                            .collect::<Vec<_>>()
                            .join("\n");
                            Ok(((!warning.is_empty()).then_some(warning), ConfigAfter::None))
                        },
                    ) {
                        window.editing.fail(error.clone());
                        window.message = error;
                    }
                }
            }
            Action::Show | Action::OpenDashboard | Action::Dashboard {..} | Action::Setting {..} | Action::Monitor {..} | Action::Library { .. } => {}
            Action::ToggleActions => {
                if window.editing.token().is_none() {
                    window.session.actions_open = !window.session.actions_open;
                    if window.session.actions_open {
                        window.editing.begin_menu();
                    }
                }
            }
            Action::Named {
                name,
                target,
                menu_token,
            } => {
                if target.is_none() && !crate::actions::global(&name) {
                    continue;
                }
                if menu_token.is_some_and(|token| {
                    token != window.editing.menu_token
                        || !window.session.actions_open
                        || ui.menu_cancelled(token)
                }) {
                    continue;
                }
                if let Some(target) = &target {
                    if !crate::actions::matches(target, &window.session, window.epoch) {
                        #[cfg(feature = "test-tools")]
                        {
                            let published = ui.view();
                            // Scalar-only diagnostics for an actual rejected
                            // intent. Never record command IDs, text or paths.
                            window.last_named_refusal = Some(json!({
                                "alias":name=="set-alias","hotkey":name=="set-hotkey",
                                "targetEpoch":target.epoch,"currentEpoch":window.epoch,
                                "targetCatalog":target.catalog_revision,"currentCatalog":window.session.results_revision(),
                                "publishedEpoch":published.epoch,"publishedCatalog":published.catalog_revision,
                                "inputEpoch":input.epoch,"inputIntent":window.input_intent,
                                "sameSelectedId":window.session.selected().is_some_and(|command|command.id==target.id),
                                "configurationRevision":state.configuration.revision,"cachedConfigurationRevision":window.config_revision,
                                "applicationsRevision":state.applications_revision,"cachedApplicationsRevision":window.applications_revision,
                                "rankingRevision":state.ranking.revision,"cachedRankingRevision":window.ranking_revision,
                                "configurationBusy":state.config_busy(),"configurationDirty":state.config_dirty
                            }));
                        }
                        window.message = "Selection changed. Try again.".into();
                        continue;
                    }
                }
                match perform_named(
                    state,
                    window,
                    &name,
                    ui,
                    menu_token,
                    input.dismissal,
                    input.deadline,
                ) {
                    Ok(warning) => window.message = warning.unwrap_or_default(),
                    Err(error) => window.message = error,
                }
            }
            Action::MenuKey {
                key,
                target,
                menu_token,
            } => {
                if !window.session.actions_open
                    || menu_token != window.editing.menu_token
                    || ui.menu_cancelled(menu_token)
                {
                    continue;
                }
                if let Some(target) = &target {
                    if !crate::actions::matches(target, &window.session, window.epoch) {
                        window.message = "Selection changed. Try again.".into();
                        continue;
                    }
                }
                let mut items = crate::actions::items(
                    window.session.selected(),
                    window.session.selected().is_some_and(|c| {
                        state
                            .configuration
                            .current()
                            .commands
                            .iter()
                            .any(|saved| saved.id == c.id)
                    }),
                );
                system::extend_items(&mut items,window);
                match key.as_str() {
                    "moveDown:" | "insertTab:" => {
                        window.editing.action_selection =
                            (window.editing.action_selection + 1).min(items.len().saturating_sub(1))
                    }
                    "moveUp:" | "insertBacktab:" => {
                        window.editing.action_selection =
                            window.editing.action_selection.saturating_sub(1)
                    }
                    "insertNewline:" => {
                        if let Some(item) = items.get(window.editing.action_selection) {
                            match perform_named(
                                state,
                                window,
                                item.id,
                                ui,
                                Some(menu_token),
                                input.dismissal,
                                input.deadline,
                            ) {
                                Ok(warning) => window.message = warning.unwrap_or_default(),
                                Err(error) => window.message = error,
                            }
                        }
                    }
                    _ => {}
                }
            }
            Action::SaveEditor {
                token,
                aliases: _,
                hotkey: _,
                attempt: _,
                command: _,
            } => {
                if window.editing.cancelled(ui) {
                    continue;
                }
                if let Err(error) = start_configuration_edit(
                    state,
                    window,
                    ui,
                    input.deadline,
                    move |store, editing, apps, _| {
                        let receipt = editing.save_with_apps(store, token, None, None, apps)?;
                        Ok((saved_warning(receipt), ConfigAfter::None))
                    },
                ) {
                    window.editing.fail(error.clone());
                    window.message = error;
                }
            }
            Action::PreviewInput { .. }
            | Action::Result { .. }
            | Action::Clipboard { .. }
            | Action::InputControl { .. }
            | Action::InputMenu { .. }
            | Action::SubmitInput { .. }
            | Action::SubmitTemplate { .. }
            | Action::CancelEditor { .. }
            | Action::CloseActions { .. } => {}
            Action::Key(selector) => {
                if window.editing.token().is_some() || window.session.actions_open {
                    continue;
                }
                if selector == "cancelOperation:" {
                    if let Some(t) = window.results.token() {
                        ui.cancel_result(t);
                        window.results.clear();
                    }
                    window.visibility_intent = Some(input.received);
                }
                if window.session.library.is_some() && selector == "insertNewline:" {
                    if let Err(error) = perform_named(
                        state,
                        window,
                        "edit",
                        ui,
                        None,
                        input.dismissal,
                        input.deadline,
                    ) {
                        window.message = error;
                    }
                    continue;
                }
                if selector == "insertTab:"
                    && window
                        .session
                        .selected()
                        .is_some_and(|c| c.input_prompt.is_some())
                {
                    if let Err(error) = admit_native(state, window, ui, input.dismissal) {
                        window.message = error;
                    }
                    continue;
                }
                window.session.key(&selector, false);
                if window.session.clipboard_mode&&matches!(selector.as_str(),"moveUp:"|"moveDown:") {
                    window.clipboard_selection=window.session.selection.map_or(0,|i|i as isize);
                }
                if matches!(selector.as_str(), "moveUp:" | "moveDown:")
                    && window.session.library.is_none()
                    && window.session.launcher_scope.is_none()
                    && !window.session.clipboard_mode
                    && window.utility.mode.is_none()
                {
                    ui.note_palette_selection();
                }
                if window.session.activated_id.take().is_some() {
                    window.session.visible = true;
                    if let Err(error) = admit_native(state, window, ui, input.dismissal) {
                        window.message = error;
                    }
                }
            }
            Action::Select {
                id,
                catalog_revision,
                epoch: view_epoch,
            } => {
                if window.editing.token().is_some()
                    || catalog_revision != window.session.results_revision()
                    || view_epoch != window.epoch
                {
                    continue;
                }
                let index = id.and_then(|id| window.session.rows().position(|c| c.id == id));
                window.session.select_row(index);
                if window.session.clipboard_mode {window.clipboard_selection=index.map_or(-1,|i|i as isize);}
                if window.session.library.is_none()
                    && window.session.launcher_scope.is_none()
                    && !window.session.clipboard_mode
                    && window.utility.mode.is_none()
                {
                    ui.note_palette_selection();
                }
            }
            Action::Activate {
                id,
                catalog_revision,
                epoch: view_epoch,
            } => {
                if window.editing.token().is_some()
                    || catalog_revision != window.session.results_revision()
                    || view_epoch != window.epoch
                {
                    continue;
                }
                let index = window.session.rows().position(|c| c.id == id);
                if index.is_some() {
                    window.session.select_row(index);
                    if window.session.library.is_some() {
                        if let Err(error) = perform_named(
                            state,
                            window,
                            "edit",
                            ui,
                            None,
                            input.dismissal,
                            input.deadline,
                        ) {
                            window.message = error;
                        }
                    } else if let Err(error) = admit_native(state, window, ui, input.dismissal) {
                        window.message = error;
                    }
                }
            }
        }
    }

    utilities::refresh(state,window,ui);
    false
}
fn ranking(state: &SharedState) -> Value {
    json!({"entries":state.ranking.entries().len(),"limit":512,"retentionDays":90})
}
fn dispatch(
    state: &mut SharedState,
    window: &mut WindowRuntime,
    request: &Value,
    observer: &ConfigurationObserver,
    headless: bool,
) -> Value {
    let result = (|| -> Result<Value, String> {
        match request["op"].as_str() {
            Some("performance") => Ok(json!({"ok":true,"result":crate::performance_trace::shared().snapshot()})),
            Some("performance.reset") => {
                crate::performance_trace::shared().reset();
                Ok(json!({"ok":true,"result":crate::performance_trace::shared().snapshot()}))
            }
            Some("status") => {
                let config = state.configuration.current();
                let permissions=state.permissions.read(!headless&&crate::runtime_health::Requirements::from_config(config).input());
                let hooks = state
                    .keyboard
                    .as_ref()
                    .map(|k| k.hook_health())
                    .unwrap_or_default();
                let mut response = json!({"ok":true,"result":{"name":"Ruycast","version":rustcast_core::product_version::CURRENT,"implementation":"rust","productionReady":false,
                    "pid":std::process::id(),"profile":state.configuration.paths.root,"configPath":state.configuration.file,
                    "configDirectory":state.configuration.file.parent(),"headless":headless,"commands":config.commands.len(),"discoveredApps":state.applications.len(),"appDiscoveryPending":state.discovery_pending,"appDiscoveryError":state.discovery_error,
                    "configError":state.configuration.last_error,"health":if state.healthy(headless,Some(&permissions)) {"healthy"}else{"needs-attention"},"accessibility":permissions.accessibility,"inputMonitoring":permissions.input_monitoring,
                    "scriptsRunning":state.execution.script_count(),"openRequestsRunning":state.execution.count()-state.execution.script_count()-state.execution.snippet_count(),"snippetRequestsRunning":state.execution.snippet_count(),"snippetTransactionsPending":crate::paste::pending_tickets(),"liveExecutionWorkers":state.execution.live_workers(),"scriptCapacity":crate::execution::LIMIT,"cliScriptCapacity":crate::execution::CLI_LIMIT,"commandLogError":state.activity.storage_error,"configurationObserver":observer.is_running(),"configurationRevision":state.configuration.revision,"configurationUpdating":state.config_busy() || state.config_dirty,
                    "hotkeyWarnings":if state.configuration.needs_effects() {state.configuration.last_error.iter().collect::<Vec<_>>()}else{vec![]},"keyboardActivationPending":state.configuration.needs_effects(),"keyboard":if state.keyboard.is_some(){"Carbon"}else{"Off"},"hyper":hooks.hyper,"modifierTaps":hooks.modifier_taps,"snippetExpansion":state.keyboard.as_ref().map(|k|k.keyword_status()).unwrap_or_else(||"Off".into()),"snippetExpansionComplete":false,
                    "theme":config.theme.as_deref().unwrap_or("system"),"menuBarVisible":config.menu_bar_visible.unwrap_or(true),
                    "separateCommandKeys":config.separate_command_keys.unwrap_or(true),"clipboardCaptureActive":!headless && config.clipboard.enabled && state.clipboard.as_ref().is_some_and(|s|!s.paused)}});
                response["result"]["clipboardEnabled"]=json!(config.clipboard.enabled);
                #[cfg(feature = "test-tools")]
                { response["result"]["lastNamedRefusal"] = json!(window.last_named_refusal.as_ref()); }
                response["result"]["clipboardPaused"]=json!(state.clipboard.as_ref().is_some_and(|s|s.paused));
                response["result"]["clipboardCount"]=json!(state.clipboard.as_ref().map_or(0,|s|s.items.len()));
                response["result"]["clipboardCleanupError"]=json!(state.clipboard.as_ref().and_then(|s|s.cleanup_error.clone()));
                response["result"]["clipboardPauseError"]=json!(state.clipboard.as_ref().and_then(|s|s.pause_error.clone()));
                response["result"]["sidedShortcuts"] = json!(hooks.sided);
                response["result"]["hidStatus"] = json!(hooks.status);
                response["result"]["hidGeneration"] = json!(hooks.generation);
                response["result"]["hidRecoveryPending"] = json!(hooks.recovering);
                response["result"]["keywordGeneration"] = json!(state
                    .keyboard
                    .as_ref()
                    .map(|k| k.keyword_generation())
                    .unwrap_or(0));
                response["result"]["keyboardGeneration"] =
                    json!(state.configuration.keyboard_generation());
                response["result"]["nativeKeyboardService"] = json!(state.keyboard.is_some());
                response["result"]["hotkeyEventsDropped"] =
                    json!(state.keyboard.as_ref().map(|k| k.dropped()).unwrap_or(0));
                response["result"]["keyboardError"] =
                    json!(state.keyboard.as_ref().and_then(|k| k.error()));
                response["result"]["recordingShortcut"] =
                    json!(state.keyboard.as_ref().is_some_and(|k| k.recording()));
                Ok(response)
            }
            Some("usage") => Ok(json!({"ok":true,"result":state.activity.counts})),
            Some("analytics") => Ok(
                json!({"ok":true,"result":state.analytics.client().snapshot(),"storageError":state.analytics.client().storage_error()}),
            ),
            Some("analytics.export") => Ok(
                json!({"ok":true,"result":state.analytics.client().snapshot().export(rustcast_core::product_version::CURRENT)}),
            ),
            Some("analytics.settings") => {
                state.analytics.client().configure(
                    crate::analytics::request_bool(&request["enabled"]),
                    crate::analytics::request_int(&request["retentionDays"]),
                )?;
                Ok(json!({"ok":true}))
            }
            Some("analytics.reset") => {
                if state.diagnostics.busy() { return Err("Diagnostics export is still in progress. Try resetting analytics afterward.".into()); }
                if crate::analytics::request_bool(&request["confirmed"]) != Some(true) {
                    return Err("Confirm resetting analytics.".into());
                }
                state.analytics.client().reset();
                Ok(json!({"ok":true}))
            }
            Some("logs") => Ok(
                json!({"ok":true,"result":state.activity.entries(request),"path":state.activity.path(),"storageError":state.activity.storage_error}),
            ),
            Some("logs.path") => Ok(json!({"ok":true,"result":{"path":state.activity.path()}})),
            Some("logs.clear") => {
                if request["confirmed"] != true {
                    return Err("Confirm clearing command logs.".into());
                }
                state.activity.clear();
                Ok(
                    json!({"ok":state.activity.storage_error.is_none(),"storageError":state.activity.storage_error}),
                )
            }
            Some("commands") => {
                Ok(json!({"ok":true,"result":state.configuration.current().commands}))
            }
            Some("search") => {
                let query = request["query"].as_str().unwrap_or("");
                let mode = request["mode"].as_str().unwrap_or("commands");
                if !["commands", "snippets", "quicklinks"].contains(&mode) {
                    return Err(
                        "Search mode is not yet implemented in this Rust development host.".into(),
                    );
                }
                let limit = request["limit"].as_i64().unwrap_or(100);
                let commands = system::local(state, query, 0)
                    .into_iter()
                    .filter(|c| match mode {
                        "snippets" => c.kind == "snippet",
                        "quicklinks" => c.kind == "url" || c.kind == "path",
                        _ => true,
                    })
                    .take(if limit <= 0 {
                        usize::MAX
                    } else {
                        limit as usize
                    })
                    .collect::<Vec<_>>();
                Ok(json!({"ok":true,"result":commands}))
            }
            Some("execute") => {
                let id = request["id"].as_str().ok_or("Missing command id.")?;
                let (command, _) = resolve_command(state, window, id)?;
                if command.enabled == Some(false) {
                    return Err("Command is disabled.".into());
                }
                if request["dryRun"] == true {
                    Ok(json!({"ok":true,"result":command,"dryRun":true}))
                } else {
                    Err("Execution is not yet implemented in this Rust development host.".into())
                }
            }
            Some("reload" | "config.directory") => {
                Err("Configuration mutations must use the background writer.".into())
            }
            Some("search-ranking") => Ok(json!({"ok":true,"result":ranking(state)})),
            Some("search-ranking.reset") => {
                if request["confirmed"] != true {
                    return Err("Confirm search ranking reset.".into());
                }
                let receipt = state.ranking.reset()?;
                window.sync(state);
                Ok(json!({"ok":true,"result":ranking(state),"durabilityWarning":receipt.warning}))
            }
            Some("show") => Err("The palette is unavailable in headless mode.".into()),
            Some("quit") => Ok(json!({"ok":true,"result":{"state":"stopping"}})),
            _ => Err("Unknown or not yet migrated operation.".into()),
        }
    })();
    result.unwrap_or_else(|error| json!({"ok":false,"error":error}))
}

fn perform_named(
    state: &mut SharedState,
    window: &mut WindowRuntime,
    name: &str,
    ui: &Arc<crate::connected::Connection>,
    origin_menu: Option<u64>,
    dismissal: u64,
    deadline: Instant,
) -> Result<Option<String>, String> {
    if window.editing.token().is_some() {
        return Ok(None);
    }
    let command = window.session.selected().cloned();
    let items = crate::actions::items(
        command.as_ref(),
        command.as_ref().is_some_and(|c| {
            state
                .configuration
                .current()
                .commands
                .iter()
                .any(|saved| saved.id == c.id)
        }),
    );
    if name=="safari-access" && window.session.library.is_none(){system::consent(state,window,ui)?;return Ok(None);}
    if name=="refresh-system" && window.session.library.is_none() {
        state.system.service.refresh();window.system.refresh();window.message.clear();window.session.actions_open=false;return Ok(None);
    }
    if !items.iter().any(|item| item.id == name) {
        return Err("Action is unavailable.".into());
    }
    window.session.actions_open = false;
    match name {
        "open" => {
            admit_native(state, window, ui, dismissal)?;
            Ok(None)
        }
        "new-snippet" | "new-quicklink" | "new-script" => {
            if ui.latest_dismissal() != dismissal
                || origin_menu.is_some_and(|token| ui.menu_cancelled(token))
            {
                return Ok(None);
            }
            if Instant::now() >= deadline {
                return Err("Creation request expired. Try again.".into());
            }
            window.editing.create(
                &state.configuration,
                match name {
                    "new-snippet" => "snippet",
                    "new-script" => "script",
                    _ => "url",
                },
                None,
            )?;
            window.editing.bind_origin_menu(origin_menu);
            if name == "new-quicklink" {
                start_creation(state, window, ui, origin_menu, dismissal, deadline);
            }
            Ok(None)
        }
        "set-alias" | "set-hotkey" | "remove" | "edit" => {
            if let Some(command) = command {
                window.editing.open(
                    &state.configuration,
                    &command,
                    match name {
                        "set-alias" => crate::actions::Mode::Alias,
                        "set-hotkey" => crate::actions::Mode::Hotkey,
                        "edit" => crate::actions::Mode::Edit,
                        _ => crate::actions::Mode::Remove,
                    },
                );
                window.editing.bind_origin_menu(origin_menu);
            }
            Ok(None)
        }
        "settings" => {
            if ui.is_management {
                ui.action(Action::Show);
            } else if let Some(peer) = ui.peer() {
                peer.action(Action::Show);
            } else {
                return Err("Settings window is unavailable.".into());
            }
            Ok(None)
        }
        "disable" | "enable" => {
            let command = command.ok_or("No command selected")?;
            let enabled = name == "enable";
            start_configuration_edit(state, window, ui, deadline, move |store, _, apps, _| {
                let receipt =
                    crate::actions::set_enabled_with_apps(store, &command, enabled, apps)?;
                Ok((receipt.warning, ConfigAfter::None))
            })?;
            Ok(None)
        }
        _ => Err("This action is unavailable in the development build.".into()),
    }
}


fn clipboard_command(c:&rustcast_core::Clip)->rustcast_core::Command {
    let title=if c.content_type.as_deref()==Some("image"){String::from("Image")}
    else if let Some(files)=c.file_urls.as_ref().filter(|_|c.content_type.as_deref()==Some("files")){
        let name=files.first().and_then(|raw|objc2_foundation::NSURL::URLWithString(&objc2_foundation::NSString::from_str(raw))).and_then(|url|url.lastPathComponent()).map(|s|s.to_string()).unwrap_or_else(||c.text.clone());
        format!("{name}{}",if files.len()>1{format!(" +{}",files.len()-1)}else{String::new()})
    }else{c.text.replace(['\n','\r']," ").chars().take(240).collect::<String>()};
    serde_json::from_value(json!({"id":format!("clipboard:{}",c.id),"title":title,"kind":"snippet","target":c.text,"aliases":[]})).expect("clipboard command schema")
}
fn show_launcher_scope(window: &mut WindowRuntime, ui: &Connection, mode: &str, query: Option<&str>, received: Instant) -> Result<(), String> {
    let scope = crate::session::LauncherScope::parse(mode).ok_or("Unknown launcher mode.")?;
    window.show(query.or(Some("")), received, ui)?;
    window.session.launcher_scope = Some(scope);
    let accepted_query = window.session.query.clone();
    window.session.set_query(&accepted_query);
    window.stamp = None;
    Ok(())
}
fn show_clipboard(state:&SharedState,window:&mut WindowRuntime,ui:&Connection,received:Instant)->Result<(),String>{
    let return_query=if window.session.clipboard_mode{window.clipboard_return_query.clone()}else{Some(window.session.query.clone())};
    window.show(Some(""),received,ui)?;window.clipboard_return_query=return_query;window.session.clipboard_mode=true;window.clipboard_selection=0;window.clipboard_query.clear();window.clipboard_results_revision=None;window.clipboard_revision=u64::MAX;window.sync(state);window.session.set_query("");window.capture_clipboard_results();window.message.clear();window.stamp=None;Ok(())
}
fn clipboard_request(state:&mut SharedState,window:&mut WindowRuntime,ui:Option<&Arc<Connection>>,request:crate::control::Envelope){
    let op=request.value["op"].as_str().unwrap_or("");
    if matches!(op,"clipboard.copy"|"clipboard.paste") {
        let result=(||->Result<_,String>{let store=state.clipboard.as_ref().ok_or("Clipboard history unavailable.")?;let id=request.value["id"].as_str().ok_or("Missing clipboard id.")?;let clip=store.items.iter().find(|c|c.id==id).ok_or("Clipboard item not found.")?;let mut command=clipboard_command(clip);if op=="clipboard.copy"{command.target="copy-only".into();}Ok((command,store.revision))})();
        match result {Ok((command,revision))=>{let ctx=request.context.clone();let deadline=Some(ctx.deadline());let hide=if op=="clipboard.paste"{ui.map(|ui|SnippetHideOrigin::capture(ui,None))}else{None};let check=hide.clone();let live=Arc::new(move||ctx.is_live()&&check.as_ref().is_none_or(SnippetHideOrigin::live));let copy=op=="clipboard.copy";let _=start_snippet(state,command,json!({"_copyOnly":copy}),ExecutionDestination::Cli(request),ExecutionOrigin::Clipboard(revision),None,live,deadline,hide);},Err(error)=>{let _=request.respond(json!({"ok":false,"error":error}));}}return;
    }
    if !matches!(op,"clipboard.list"|"clipboard.export"){if let Some(store)=&mut state.clipboard{store.bind_publication(Some(request.context.clipboard_publication()));}}
    let response=(||->Result<Value,String>{
        let settings=&state.configuration.current().clipboard;let store=state.clipboard.as_mut().ok_or("Clipboard history unavailable.")?;
        let live=||request.context.is_live();
        match op {
            "clipboard.list"|"clipboard.export"=>Ok(json!({"ok":true,"result":store.list(request.value["query"].as_str().unwrap_or(""))})),
            "clipboard.import"=>store.import(&request.value["items"],settings,live),
            "clipboard.clear"|"clipboard.delete"=>{let id=if op=="clipboard.delete"{Some(request.value["id"].as_str().ok_or("Missing clipboard id.")?)}else{None};let receipt=store.delete(id,settings,live)?;Ok(json!({"ok":true,"result":{"count":store.items.len()},"durabilityWarning":receipt.warning,"cleanupError":store.cleanup_error}))},
            "clipboard.pause"=>{let receipt=store.pause(request.value["paused"].as_bool().unwrap_or(true),live)?;Ok(json!({"ok":true,"result":{"paused":store.paused},"durabilityWarning":receipt.warning}))},
            _=>Err("Unknown clipboard operation.".into())
        }
    })().unwrap_or_else(|error|json!({"ok":false,"error":error}));
    if let Some(store)=&mut state.clipboard{store.bind_publication(None);}
    window.stamp=None;let _=request.respond(response);
}
fn clipboard_input(state:&mut SharedState,window:&mut WindowRuntime,ui:&Arc<Connection>,input:&crate::connected::Input)->bool{
    let mut operation=None;
    match &input.action {
        // Clipboard rows project private payloads as transient commands. They
        // must never enter the saved-command editor or configuration writer.
        Action::Named{name,..} if matches!(name.as_str(),"set-alias"|"set-hotkey"|"edit"|"remove"|"enable"|"disable")=>return true,
        Action::Clipboard{operation:op,id,revision,epoch,dismissal}=>{
            let current=state.clipboard.as_ref().map(|s|s.revision);
            if current!=Some(*revision)||*epoch!=window.epoch||*dismissal!=window.dismissal||*dismissal!=ui.latest_dismissal()||window.session.selected().and_then(|c|c.id.strip_prefix("clipboard:")).map(str::to_owned)!=*id{return true}
            operation=Some(op.as_str());
        },
        Action::Key(key) if key=="insertNewline:"=>operation=Some("paste"),
        Action::Key(key) if key=="deleteBackward:" && window.session.query.is_empty()=>operation=Some("delete"),
        Action::Activate{id,catalog_revision,epoch}=>{if *catalog_revision==window.session.results_revision()&&*epoch==window.epoch{let index=window.session.rows().position(|c|&c.id==id);window.session.select_row(index);operation=Some("paste");}},
        Action::Named{name,target,menu_token} if name.starts_with("clipboard.")=>{
            if menu_token.is_some_and(|t|t!=window.editing.menu_token||!window.session.actions_open||ui.menu_cancelled(t)){return true}
            if target.as_ref().is_some_and(|t|t.epoch!=window.epoch||t.catalog_revision!=window.session.results_revision()||window.session.selected().is_none_or(|c|c.id!=t.id)){return true}
            operation=Some(name.trim_start_matches("clipboard."));
        },
        Action::MenuKey{key,target,menu_token}=>{
            if *menu_token!=window.editing.menu_token||!window.session.actions_open||ui.menu_cancelled(*menu_token)||target.as_ref().is_some_and(|t|t.epoch!=window.epoch||t.catalog_revision!=window.session.results_revision()||window.session.selected().is_none_or(|c|c.id!=t.id)){return true}
            let items=if window.session.selected().is_some(){&["paste","copy","delete","settings"][..]}else{&["settings"][..]};
            match key.as_str(){"moveDown:"|"insertTab:"=>window.editing.action_selection=(window.editing.action_selection+1).min(items.len()-1),"moveUp:"|"insertBacktab:"=>window.editing.action_selection=window.editing.action_selection.saturating_sub(1),"insertNewline:"=>operation=items.get(window.editing.action_selection).copied(),_=>{}}
        },
        Action::ToggleActions=>{window.session.actions_open = !window.session.actions_open;if window.session.actions_open{window.editing.begin_menu();}return true},
        _=>return false
    }
    if let Some(operation)=operation {
        if !["back","settings","pause","delete","copy","paste"].contains(&operation){return true;}
        window.session.actions_open=false;
        let result=(||->Result<(),String>{
            if operation=="back" {let query=window.clipboard_return_query.take().unwrap_or_default();window.show(Some(&query),input.received,ui)?;window.sync(state);window.stamp=None;return Ok(())}
            if operation=="settings" {let peer=ui.peer().ok_or("Settings window is unavailable.")?;peer.action(Action::Show);window.session.visible=false;window.stamp=None;return Ok(())}
            if operation=="pause" {let s=state.clipboard.as_mut().ok_or("Clipboard history unavailable.")?;s.pause(!s.paused,||!ui.stop_requested())?;window.stamp=None;return Ok(())}
            let command=window.session.selected().cloned().ok_or("Select a clipboard item.")?;
            if operation=="delete"{let store=state.clipboard.as_mut().ok_or("Clipboard history unavailable.")?;store.delete(command.id.strip_prefix("clipboard:"),&state.configuration.current().clipboard,||!ui.stop_requested())?;window.sync(state);return Ok(())}
            let copy=operation=="copy";let mut command=command;if copy{command.target="copy-only".into();}
            start_native_snippet(state,window,ui,input.dismissal,command,json!({"_copyOnly":copy}),None)
        })();if let Err(error)=result{window.message=error;window.stamp=None;}
    }true
}

#[cfg(test)]
mod inline_tests {
    use super::*;
    include!("runtime_private_tests.rs");
    #[test]
    fn clipboard_rows_cannot_open_saved_command_editors_or_mutate_configuration() {
        let (_temp, mut state, _, _, _) = setup();
        let settings = state.configuration.current().clipboard.clone();
        let mut store = crate::clipboard_history::Store::open(&state.configuration.paths.root, &settings).unwrap();
        store.capture(crate::clipboard_history::Payload::Text("Synthetic private clipboard text".into()), "fixture".into(), crate::clipboard_history::now(), &settings, || true).unwrap();
        state.clipboard = Some(store);
        let (tx, _rx) = mpsc::sync_channel(32);
        let ui = Connection::new(tx, true);
        let mut window = WindowRuntime::new(crate::session::Session::new(vec![]), Default::default(), &state);
        show_clipboard(&state, &mut window, &ui, Instant::now()).unwrap();
        window.publish(&state, &ui);
        let selected = window.session.selected().unwrap().id.clone();
        assert!(selected.starts_with("clipboard:"));
        let config_file = state.configuration.paths.config_file().unwrap();
        let before = std::fs::read(&config_file).unwrap();
        let revision = state.configuration.revision;
        let clip_revision = state.clipboard.as_ref().unwrap().revision;
        for name in ["set-alias", "set-hotkey", "edit", "remove", "enable", "disable"] {
            ui.action(Action::Named { name: name.into(), target: None, menu_token: None });
            process_inputs(&mut state, &mut window, &ui);
            assert!(window.editing.token().is_none(), "clipboard action opened editor: {name}");
            assert_eq!(window.session.selected().unwrap().id, selected);
            assert_eq!(state.configuration.revision, revision);
            assert_eq!(state.clipboard.as_ref().unwrap().revision, clip_revision);
            assert_eq!(std::fs::read(&config_file).unwrap(), before);
        }
    }
    #[test]
    fn clipboard_menu_delete_is_selection_bound_and_rejects_stale_menu() {
        let (_temp, mut state, _, _, _) = setup();
        let settings=state.configuration.current().clipboard.clone();
        let mut store=crate::clipboard_history::Store::open(&state.configuration.paths.root,&settings).unwrap();
        store.capture(crate::clipboard_history::Payload::Text("Synthetic menu item".into()),"fixture".into(),crate::clipboard_history::now(),&settings,||true).unwrap();
        state.clipboard=Some(store);
        let (tx,_rx)=mpsc::sync_channel(32);
        let ui=Connection::new(tx,true);
        let mut window=WindowRuntime::new(crate::session::Session::new(vec![]),Default::default(),&state);
        show_clipboard(&state,&mut window,&ui,Instant::now()).unwrap();
        window.publish(&state,&ui);
        ui.action(Action::ToggleActions);
        process_inputs(&mut state,&mut window,&ui);
        window.publish(&state,&ui);
        let view=ui.view();
        assert_eq!(view.action_items.iter().map(|i|i.title.as_str()).collect::<Vec<_>>(),["Paste","Copy","Delete","Settings"]);
        assert!(view.actions_open);
        let stale=Action::Named{name:"clipboard.delete".into(),target:view.action_target.clone(),menu_token:Some(view.menu_token+1)};
        ui.action(stale);
        process_inputs(&mut state,&mut window,&ui);
        assert_eq!(state.clipboard.as_ref().unwrap().items.len(),1);
        ui.action(Action::Named{name:"clipboard.delete".into(),target:view.action_target.clone(),menu_token:Some(view.menu_token)});
        process_inputs(&mut state,&mut window,&ui);
        assert!(state.clipboard.as_ref().unwrap().items.is_empty());
        assert!(!window.session.actions_open);
    }
    fn gesture_consent_fixture() -> (Temp, SharedState, WindowRuntime, Arc<Connection>, Arc<crate::system_search::fixture::Fixture>) {
        let (temp, state, _, _, _) = setup();
        let ui = Connection::new(state.events.clone(), true);
        let fixture = state.system.fixture.as_ref().unwrap().clone();
        let window = WindowRuntime::new(crate::session::Session::new(state.configuration.current().commands.clone()), Default::default(), &state);
        (temp, state, window, ui, fixture)
    }
    fn gesture_consent_count(fixture: &crate::system_search::fixture::Fixture) -> usize {
        fixture.effects.lock().unwrap().iter().filter(|effect| effect.as_str() == "consent").count()
    }
    #[test]
    fn launcher_user_gesture_show_requests_owned_consent_once() {
        let (_temp, mut state, mut window, ui, fixture) = gesture_consent_fixture();
        assert_eq!(gesture_consent_count(&fixture), 0);
        ui.action(Action::Show);
        process_inputs(&mut state, &mut window, &ui);
        assert!(window.session.visible);
        let until = Instant::now() + Duration::from_millis(250);
        while gesture_consent_count(&fixture) == 0 && Instant::now() < until { std::thread::sleep(Duration::from_millis(1)); }
        assert_eq!(gesture_consent_count(&fixture), 1);
        assert!(window.message.is_empty());
    }
    #[test]
    fn launcher_non_gesture_and_management_routes_never_request_owned_consent() {
        for route in ["cli-show", "query", "management", "stale-show"] {
            let (_temp, mut state, mut window, ui, fixture) = gesture_consent_fixture();
            if route == "management" {
                let managed = Connection::new_management(state.events.clone(), true);
                managed.action(Action::Show);
                process_inputs(&mut state, &mut window, &managed);
                assert!(window.session.visible);
            } else if route == "stale-show" {
                window.visibility_intent = Some(Instant::now() + Duration::from_secs(1));
                ui.action(Action::Show);
                process_inputs(&mut state, &mut window, &ui);
                assert!(!window.session.visible);
            } else {
                window.show(Some("Owned"), Instant::now(), &ui).unwrap();
                if route == "query" {
                    ui.query("other");
                    process_inputs(&mut state, &mut window, &ui);
                    system::tick(&mut window, &state, &ui);
                }
                assert!(window.session.visible);
            }
            std::thread::sleep(Duration::from_millis(20));
            assert_eq!(gesture_consent_count(&fixture), 0, "{route}");
        }
    }
    #[test]
    fn launcher_user_gesture_resets_hidden_clipboard_query_editor_and_scope_before_consent() {
        let (_temp, mut state, mut window, ui, fixture) = gesture_consent_fixture();
        window.session.clipboard_mode = true;
        window.session.launcher_scope = Some(crate::session::LauncherScope::Snippets);
        window.session.set_query("Owned previous query");
        window.session.actions_open = true;
        window.utility.mode = Some("text-history".into());
        let command = state.configuration.current().commands[0].clone();
        window.editing.open(&state.configuration, &command, crate::actions::Mode::Alias);
        assert!(window.editing.token().is_some());
        ui.action(Action::Show);
        process_inputs(&mut state, &mut window, &ui);
        assert!(window.session.visible);
        assert!(!window.session.clipboard_mode);
        assert!(window.session.launcher_scope.is_none());
        assert!(window.utility.mode.is_none());
        assert!(window.editing.token().is_none());
        assert!(window.session.query.is_empty());
        assert!(!window.session.actions_open);
        let until = Instant::now() + Duration::from_millis(250);
        while gesture_consent_count(&fixture) == 0 && Instant::now() < until { std::thread::sleep(Duration::from_millis(1)); }
        assert_eq!(gesture_consent_count(&fixture), 1);
    }
    #[test]
    fn launcher_user_gesture_toggles_visible_commands_without_requesting_consent() {
        let (_temp, mut state, mut window, ui, fixture) = gesture_consent_fixture();
        window.session.visible = true;
        ui.action(Action::Show);
        process_inputs(&mut state, &mut window, &ui);
        assert!(!window.session.visible);
        std::thread::sleep(Duration::from_millis(20));
        assert_eq!(gesture_consent_count(&fixture), 0);
    }
    #[test]
    fn launcher_user_gesture_keeps_show_when_owned_consent_is_busy() {
        let (_temp, mut state, mut window, ui, fixture) = gesture_consent_fixture();
        system::consent(&state, &mut window, &ui).unwrap();
        let until = Instant::now() + Duration::from_secs(1);
        while gesture_consent_count(&fixture) == 0 && Instant::now() < until { std::thread::sleep(Duration::from_millis(1)); }
        assert_eq!(gesture_consent_count(&fixture), 1);
        ui.action(Action::Show);
        process_inputs(&mut state, &mut window, &ui);
        assert!(window.session.visible);
        assert!(window.message.is_empty());
        std::thread::sleep(Duration::from_millis(20));
        assert_eq!(gesture_consent_count(&fixture), 1);
    }
    pub(super) struct Temp(std::path::PathBuf);
    impl Drop for Temp {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    pub(super) fn setup() -> (
        Temp,
        SharedState,
        WindowRuntime,
        Arc<Connection>,
        crate::actions::Target,
    ) {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let root = std::env::var_os("RUYCAST_NATIVE_TEST_FIXTURE_DIR").map(std::path::PathBuf::from)
            .unwrap_or_else(||std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target"))
            .join(format!(
                "ir{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            ));
        std::fs::create_dir(&root).unwrap();
        let temp = Temp(root.clone());
        let HostState {
            configuration,
            ranking,
            ..
        } = HostState::open(ProfilePaths::new(root.join("profile"), root, None))
        .unwrap();
        let (execution_tx, _execution_rx) = mpsc::sync_channel(32);
        let activity = crate::activity::Activity::open(&configuration.paths.root).unwrap();
        let mut shared = SharedState {
            permissions:crate::permissions::Source::new(true),
            clipboard: None,
            system: system::Shared::new(true),
            utility:utilities::State::new(&configuration.paths,true,None,execution_tx.clone()).unwrap(),
            keyboard: None,
            deferred_hotkey: None,
            writer: None,
            config_context: None,
            config_dirty: false,
            app_choices: Default::default(),
            events: execution_tx.clone(),
            fixture: true,
            services: None,
            open_validation_stamp: None,
            execution: crate::execution::Manager::new(
                configuration.paths.home.clone(),
                execution_tx,
            ),
            activity,
            diagnostics: Default::default(),
        analytics: crate::analytics::Writer::open(&configuration.paths.root, None).unwrap(),
            configuration,
            ranking,
            applications: vec![],
            applications_revision: 0,
            discovery_pending: false,
            discovery_error: None,
        };
        let mut w = WindowRuntime::new(
            crate::session::Session::management(shared.configuration.current().commands.clone()),
            Default::default(),
            &shared,
        );
        w.session.visible = true;
        let a = w
            .session
            .rows()
            .find(|c| c.id == "app.finder")
            .unwrap()
            .clone();
        w.editing
            .transition_inline(
                &mut shared.configuration,
                &a,
                crate::actions::Mode::Alias,
                None,
                || true,
            )
            .unwrap();
        let (tx, _rx) = mpsc::sync_channel(1);
        let ui = Connection::new_management(tx, true);
        let target = crate::actions::Target {
            id: "app.safari".into(),
            catalog_revision: w.session.results_revision(),
            epoch: 0,
        };
        (temp, shared, w, ui, target)
    }
    fn install_writer(state: &mut SharedState) -> crate::control::ControlServer {
        let server = crate::control::ControlServer::start(
            &state.configuration.paths.root,
            state.events.clone(),
            crate::control::Limits::default(),
        )
        .unwrap();
        state.writer = Some(crate::configuration_jobs::Writer::new(
            server.profile_lease(),
            state.events.clone(),
            None,
        ));
        server
    }
    fn settle(state: &mut SharedState, management: &mut WindowRuntime) {
        let mut palette = WindowRuntime::new(
            crate::session::Session::new(vec![]),
            Default::default(),
            state,
        );
        let deadline = Instant::now() + Duration::from_secs(3);
        while state.config_busy() || state.config_dirty {
            finish_configuration(state, &mut palette, management);
            assert!(Instant::now() < deadline, "configuration did not settle");
            std::thread::sleep(Duration::from_millis(2));
        }
    }
    #[cfg(feature="test-tools")]
    fn process_request(operation:OwnedOperation)->OwnedRequest {
        let (response,_)=mpsc::sync_channel(1);
        OwnedRequest {operation,deadline:Instant::now()+Duration::from_secs(5),cancelled:Arc::new(std::sync::atomic::AtomicBool::new(false)),response}
    }
    #[cfg(feature="test-tools")]
    #[test]
    fn owned_process_recency_uses_real_persistent_store_and_rejects_expired_mutation() {
        let (_temp,mut state,_,_,_)=setup();
        let epoch=1_793_700_000;
        let mut request=process_request(OwnedOperation::StageProcessRecency(epoch));
        request.deadline=Instant::now()+crate::native_f1::Lane::Functional(crate::native_f1::Functional::ProcessSort1000).budget();
        owned_process_recency(&mut state,&request,epoch).unwrap();
        assert_eq!(state.ranking.entries().len(),512);
        assert_eq!(state.ranking.entries()["fixture.0"],(epoch-511) as f64);
        assert_eq!(state.ranking.entries()["fixture.511"],epoch as f64);
        assert_eq!(state.ranking.revision,513);
        let before=std::fs::read(state.configuration.paths.root.join("search-ranking.json")).unwrap();
        request.cancelled.store(true,std::sync::atomic::Ordering::Release);
        assert!(owned_process_recency(&mut state,&request,epoch).is_err());
        assert_eq!(std::fs::read(state.configuration.paths.root.join("search-ranking.json")).unwrap(),before);
        assert_eq!(state.ranking.revision,513);
    }
    #[cfg(feature="test-tools")]
    #[test]
    fn owned_process_measurements_use_actual_session_resources_sort_and_selection() {
        let (_temp,state,mut window,_,_)=setup();
        window.editing.cancel_all();
        window.session=crate::session::Session::new(vec![]);
        window.session.set_query("m");
        let (tx,_)=mpsc::sync_channel(1);
        let ui=Connection::new(tx,false);
        owned_process_operation(&mut window,&state,Some(&ui),&process_request(OwnedOperation::StageProcesses1000)).unwrap();
        assert_eq!(window.session.query,"");
        assert_eq!(window.session.row_count(),1000);
        assert_eq!(window.system.snapshot.metadata.len(),1000);
        for command in &window.system.snapshot.commands {
            assert_eq!(serde_json::to_value(state.system.command(&command.id).unwrap()).unwrap(),serde_json::to_value(command).unwrap());
        }
        assert_eq!(window.session.selected().unwrap().id,"fixture.process.96");
        let original=window.session.rows().map(|r|r.id.clone()).collect::<Vec<_>>();
        let read=owned_process_operation(&mut window,&state,Some(&ui),&process_request(OwnedOperation::CachedProcesses1000)).unwrap().unwrap();
        assert_eq!(read["result"]["visible_count"],1000);
        assert_eq!(read["result"]["selected_id_count"],"fixture.process.96".len());
        window.session.select_row(Some(30));
        let general_selected=window.session.selected().unwrap().id.clone();
        system::process_sort(&mut window,crate::processes::Sort::Memory);
        assert_eq!(window.session.selected().unwrap().id,general_selected);
        assert!(owned_process_operation(&mut window,&state,Some(&ui),&process_request(OwnedOperation::ProcessSort1000)).is_err());
        system::process_sort(&mut window,crate::processes::Sort::Cpu);
        assert_eq!(window.session.selected().unwrap().id,general_selected);
        window.session.select_row(Some(0));
        let selected=window.session.selected().unwrap().id.clone();
        assert_eq!(selected,"fixture.process.96");
        let sorted=owned_process_operation(&mut window,&state,Some(&ui),&process_request(OwnedOperation::ProcessSort1000)).unwrap().unwrap();
        assert!(sorted["result"]["visible_count"].is_null());
        assert_eq!(window.system.process_sort,crate::processes::Sort::Memory);
        assert_eq!(window.session.rows().map(|r|r.id.clone()).collect::<Vec<_>>(),(0..1000).rev().map(|i|format!("fixture.process.{i}")).collect::<Vec<_>>());
        assert_eq!(window.session.selected().unwrap().id,selected);
        // Per-sample preparation must preserve CPU/Memory alternation and the
        // selected identity, rather than silently restarting the workload.
        let correct=window.session.rows().cloned().collect::<Vec<_>>();
        let mut reordered=correct.clone();
        reordered.swap(0,1);
        window.session.set_external_results(reordered,false);
        window.session.select_row(Some(903));
        assert_eq!(window.session.selected().unwrap().id,selected);
        assert!(owned_process_operation(&mut window,&state,Some(&ui),&process_request(OwnedOperation::ValidateProcesses1000)).is_err(),"all resource IDs and sort label match, but a reordered fixture must be refused");
        assert_eq!(window.session.rows().next().unwrap().id,"fixture.process.998","validation must not repair the displaced order");
        window.session.set_external_results(correct,false);
        window.session.select_row(Some(903));
        owned_process_operation(&mut window,&state,Some(&ui),&process_request(OwnedOperation::ValidateProcesses1000)).unwrap();
        assert_eq!(window.system.process_sort,crate::processes::Sort::Memory);
        assert_eq!(window.session.selected().unwrap().id,selected);
        owned_process_operation(&mut window,&state,Some(&ui),&process_request(OwnedOperation::StageProcesses1000)).unwrap();
        assert_eq!(window.system.process_sort,crate::processes::Sort::Memory);
        owned_process_operation(&mut window,&state,Some(&ui),&process_request(OwnedOperation::ProcessSort1000)).unwrap();
        assert_eq!(window.session.rows().map(|r|r.id.clone()).collect::<Vec<_>>(),original);
        assert_eq!(window.session.selected().unwrap().id,selected);
        system::tick(&mut window,&state,&ui);
        assert!(!window.system.busy());
        assert_eq!(window.session.row_count(),1000);
        assert!(!window.session.visible);
        assert!(state.system.fixture.as_ref().unwrap().effects.lock().unwrap().is_empty());
    }
    #[cfg(feature="test-tools")]
    #[test]
    fn owned_process_measurements_reject_wrong_owner_expiry_cancel_and_changed_fixture() {
        let (_temp,state,mut window,management,_)=setup();
        window.editing.cancel_all();window.session=crate::session::Session::new(vec![]);
        let stage=process_request(OwnedOperation::StageProcesses1000);
        assert!(owned_process_operation(&mut window,&state,None,&stage).is_err());
        assert!(owned_process_operation(&mut window,&state,Some(&management),&stage).is_err());
        let (tx,_)=mpsc::sync_channel(1);let ui=Connection::new(tx,false);
        assert!(owned_process_operation(&mut window,&state,Some(&ui),&process_request(OwnedOperation::ValidateProcesses1000)).is_err());
        assert_ne!(window.system.scope,crate::system_search::Scope::Processes,"validation must not initialize or reset a displaced process fixture");
        let mut expired=process_request(OwnedOperation::StageProcesses1000);
        expired.deadline=Instant::now();
        assert!(owned_process_operation(&mut window,&state,Some(&ui),&expired).is_err());
        let cancelled=process_request(OwnedOperation::StageProcesses1000);
        cancelled.cancelled.store(true,std::sync::atomic::Ordering::Release);
        assert!(owned_process_operation(&mut window,&state,Some(&ui),&cancelled).is_err());
        assert_eq!(window.system.scope,crate::system_search::Scope::MainTabs);
        owned_process_operation(&mut window,&state,Some(&ui),&stage).unwrap();
        window.session.select_row(Some(1));
        assert_eq!(window.session.selected().unwrap().id,"fixture.process.193");
        assert!(owned_process_operation(&mut window,&state,Some(&ui),&stage).is_err());
        assert!(owned_process_operation(&mut window,&state,Some(&ui),&process_request(OwnedOperation::CachedProcesses1000)).is_err());
        window.session.select_row(Some(0));
        system::process_sort(&mut window,crate::processes::Sort::Memory);
        window.session.select_row(Some(0));
        assert_eq!(window.session.selected().unwrap().id,"fixture.process.999");
        assert!(owned_process_operation(&mut window,&state,Some(&ui),&stage).is_err());
        assert!(owned_process_operation(&mut window,&state,Some(&ui),&process_request(OwnedOperation::ProcessSort1000)).is_err());
        window.session.select_row(Some(903));
        assert_eq!(window.session.selected().unwrap().id,"fixture.process.96");
        window.system.snapshot.metadata.remove("fixture.process.5");
        let before=window.session.results_revision();
        assert!(owned_process_operation(&mut window,&state,Some(&ui),&process_request(OwnedOperation::ProcessSort1000)).is_err());
        assert_eq!(window.session.results_revision(),before);
        assert!(owned_process_operation(&mut window,&state,Some(&ui),&stage).is_err());
        assert_eq!(window.system.snapshot.metadata.len(),999);
    }
    #[test]
    fn unchanged_late_configuration_notice_keeps_fresh_admission_available() {
        let (_temp, mut state, mut management, _, _) = setup();
        let mut control = install_writer(&mut state);
        state.queue_reload(ConfigContext::Watch, Arc::new(||true), false).unwrap();
        settle(&mut state, &mut management);
        assert!(state.configuration.snapshot_is_current(Some(32_000_000)).unwrap());
        assert!(!state.configuration.needs_effects());
        let revision = state.configuration.revision;
        state.configuration_changed();
        let fresh = state.reload_watched();
        // Harvest the real worker even on the causal old-body failure.
        settle(&mut state, &mut management);
        control.stop();
        assert!(fresh.is_ok(), "A byte-identical delayed notice must not revoke fresh admission: {fresh:?}");
        assert_eq!(state.configuration.revision, revision);
    }
    #[test]
    fn late_configuration_notice_retains_changed_invalid_and_pending_reconciliation() {
        for scenario in ["changed", "invalid", "missing", "effects", "dirty", "busy"] {
            let (_temp, mut state, mut management, _, _) = setup();
            let mut control = install_writer(&mut state);
            match scenario {
                "changed" => {
                    let mut value = serde_json::to_value(state.configuration.current()).unwrap();
                    value["commands"][0]["title"] = json!("Own changed command");
                    std::fs::write(&state.configuration.file, serde_json::to_vec(&value).unwrap()).unwrap();
                }
                "invalid" => std::fs::write(&state.configuration.file, b"{").unwrap(),
                "missing" => std::fs::remove_file(&state.configuration.file).unwrap(),
                "effects" => state.configuration.require_reconciliation(),
                "dirty" => state.config_dirty = true,
                "busy" => state.queue_reload(ConfigContext::Watch, Arc::new(||true), false).unwrap(),
                _ => unreachable!(),
            }
            state.configuration_changed();
            assert!(state.config_busy(), "{scenario} must retain the existing writer path");
            assert!(state.reload_watched().is_err(), "{scenario} must not gain premature admission");
            settle(&mut state, &mut management);
            if scenario == "changed" {
                assert_eq!(state.configuration.current().commands[0].title, "Own changed command");
            }
            if matches!(scenario, "invalid" | "missing") {
                assert!(state.configuration.last_error.is_some());
            }
            control.stop();
        }
    }
    #[test]
    fn same_snapshot_notice_recovers_real_invalid_selector_error() {
        let (_temp, mut state, mut management, _, _) = setup();
        let mut control = install_writer(&mut state);
        let revision = state.configuration.revision;
        let selector = state.configuration.paths.root.join("config-location.json");
        std::fs::write(&selector, vec![b' '; 1_000_001]).unwrap();
        state.configuration_changed();
        settle(&mut state, &mut management);
        assert!(state.configuration.last_error.is_some(), "The real oversized selector must fail before recovery");
        assert_eq!(state.configuration.revision, revision);
        std::fs::write(&selector, serde_json::to_vec(&json!({"directory":state.configuration.paths.root})).unwrap()).unwrap();
        assert!(state.configuration.snapshot_is_current(Some(32_000_000)).unwrap());
        assert!(!state.configuration.needs_effects());
        assert!(!state.config_busy() && !state.config_dirty);
        state.configuration_changed();
        settle(&mut state, &mut management);
        control.stop();
        assert!(state.configuration.last_error.is_none(), "A successful same-snapshot reload must clear the previous real selector error");
        assert_eq!(state.configuration.revision, revision);
    }
    #[cfg(feature = "test-tools")]
    #[test]
    fn owned_catalog_completion_rejects_durable_write_with_pending_activation() {
        struct CommitFailure;
        impl crate::config_effects::Stage for CommitFailure {
            fn commit(&mut self)->Result<crate::config_effects::Receipt,String>{
                Err("Own catalog activation failure".into())
            }
        }
        let (_temp,mut state,mut management,_,_)=setup();
        let _server=install_writer(&mut state);
        let mut value=serde_json::from_slice::<Value>(state.configuration.snapshot()).unwrap();
        value["launcherHotkey"]=json!("cmd+a");
        let (reply,response)=mpsc::sync_channel(1);
        state.config_context=Some(ConfigContext::Owned(OwnedRequest{
            operation:OwnedOperation::StageCatalog(500),deadline:Instant::now()+Duration::from_secs(3),
            cancelled:Arc::new(std::sync::atomic::AtomicBool::new(false)),response:reply,
        }));
        state.writer.as_mut().unwrap().start(&state.configuration,Arc::new(||true),false,move|store|{
            // Inject only the platform commit failure, after the real writer
            // has acquired its ordinary actor/lease. Storage still really runs.
            store.configure_writer_effects(Arc::new(|_|Ok(Box::new(CommitFailure))),false,Arc::new(||true));
            let receipt=store.save(&value)?;
            assert!(receipt.durable,"The counterexample requires confirmed durable storage.");
            assert!(store.needs_effects());
            assert!(store.last_error.is_some());
            // This deliberately reproduces the old producer's optimistic reply.
            // The independently fresh owner-adoption guard must still reject it.
            Ok(ConfigPayload::response(json!({"ok":receipt.durable})))
        }).unwrap();
        settle(&mut state,&mut management);
        assert!(response.recv_timeout(Duration::from_secs(1)).unwrap().is_err(),"Durable storage must not admit a catalog whose activation failed.");
        assert!(state.configuration.needs_effects());
        assert!(state.configuration.last_error.as_deref().is_some_and(|e|e.contains("activation is pending")));
        assert_eq!(state.configuration.current().launcher_hotkey,"cmd+a");
        assert_eq!(std::fs::read(&state.configuration.file).unwrap(),state.configuration.snapshot());
    }
    #[cfg(feature = "test-tools")]
    #[test]
    fn owned_catalog_adoption_rechecks_late_state_and_preserves_warnings() {
        let (_temp,state,_,_,_)=setup();
        let ready=||ConfigPayload::response(json!({"ok":true}));
        assert_eq!(owned_catalog_adoption(&state.configuration,false,ready()).unwrap()["ok"],true);
        assert!(owned_catalog_adoption(&state.configuration,true,ready()).is_err());
        let mut late_pending=state.configuration.actor_copy();
        late_pending.require_reconciliation();
        assert!(late_pending.last_error.is_none());
        assert!(owned_catalog_adoption(&late_pending,false,ready()).is_err());
        assert!(late_pending.needs_effects());
        let mut late_warning=state.configuration.actor_copy();
        late_warning.last_error=Some("Own late activation warning".into());
        assert!(!late_warning.needs_effects());
        assert!(owned_catalog_adoption(&late_warning,false,ready()).is_err());
        assert_eq!(late_warning.last_error.as_deref(),Some("Own late activation warning"));
        assert_eq!(late_warning.snapshot(),state.configuration.snapshot());
        assert_eq!(late_pending.snapshot(),state.configuration.snapshot());
    }
    #[test]
    fn accepted_configuration_publishes_capture_policy_before_releasing_admission() {
        let (_temp, mut state, mut management, _, _) = setup();
        let (services, _server) = crate::main_services::channel();
        let port = services.clipboard_capture.clone();
        port.publish(&state.configuration.current().clipboard, false);
        state.services = Some(services);
        let mut control = install_writer(&mut state);
        let writer = state.writer.as_mut().unwrap();
        writer.bind_capture(&port);
        let mut store = crate::clipboard_history::Store::open_with_retention(
            &state.configuration.paths.root,
            &state.configuration.current().clipboard,
            writer.retention_phase(),
        ).unwrap();
        store.pause(true, ||true).unwrap();
        state.clipboard = Some(store);
        let mut candidate = serde_json::to_value(state.configuration.current()).unwrap();
        candidate["clipboard"]["enabled"] = json!(false);
        candidate["clipboard"]["excludedApps"] = json!(["dev.fixture.private"]);
        std::fs::write(&state.configuration.file, serde_json::to_vec(&candidate).unwrap()).unwrap();
        state.queue_reload(ConfigContext::Watch, Arc::new(||true), false).unwrap();
        settle(&mut state, &mut management);
        assert!(!port.admission_busy());
        let policy = port.policy().unwrap();
        assert!(!policy.settings.enabled);
        assert_eq!(policy.settings.excluded_apps, ["dev.fixture.private"]);
        assert!(policy.paused);
        control.stop();
    }
    #[test]
    fn configuration_origin_rejects_already_cancelled_parent_or_menu() {
        for menu in [false, true] {
            let (_temp, _state, mut window, ui, _) = setup();
            let parent = window.editing.token().unwrap();
            if menu {
                window.editing.bind_origin_menu(Some(parent));
            } else {
                window.editing.bind_origin_inline(Some(parent));
            }
            // Preserve a live child token above the separately revoked parent.
            let mut before = window.editing.view().unwrap();
            before.token += 1;
            ui.action(if menu {
                Action::CloseActions { token: parent }
            } else {
                Action::CancelEditor { token: parent }
            });
            let origin = ConfigOrigin {
                draft_live: None,
                ui: Arc::downgrade(&ui),
                epoch: ui.latest_epoch(),
                intent: ui.latest_intent(),
                dismissal: ui.latest_dismissal(),
                deadline: Instant::now() + Duration::from_secs(1),
                before: Some(before),
            };
            assert!(!origin.clocks());
        }
    }
    #[test]
    fn expired_owner_delivery_acknowledges_save_but_never_presents_worker_followup() {
        let (_temp, mut state, mut window, ui, _) = setup();
        let _server = install_writer(&mut state);
        let token = window.editing.token().unwrap();
        let deadline = Instant::now() + Duration::from_millis(200);
        let (entered, wait) = mpsc::channel();
        let (release, hold) = mpsc::channel();
        start_configuration_edit(
            &mut state,
            &mut window,
            &ui,
            deadline,
            move |store, editing, apps, _| {
                editing.update_draft(token, 1, Some("before-expiry".into()), None);
                let receipt = editing.save_with_apps(store, token, None, None, apps)?;
                editing.create(store, "snippet", None)?;
                entered.send(()).unwrap();
                hold.recv().unwrap();
                Ok((receipt.warning, ConfigAfter::None))
            },
        )
        .unwrap();
        wait.recv_timeout(Duration::from_secs(2)).unwrap();
        std::thread::sleep(
            deadline.saturating_duration_since(Instant::now()) + Duration::from_millis(2),
        );
        release.send(()).unwrap();
        settle(&mut state, &mut window);
        assert!(window.editing.token().is_none());
        assert_eq!(
            state
                .configuration
                .current()
                .commands
                .iter()
                .find(|c| c.id == "app.finder")
                .unwrap()
                .aliases,
            ["before-expiry"]
        );
    }
    #[test]
    fn expired_child_save_restores_exact_owned_input_value() {
        let (_temp, mut state, mut window, ui, _) = setup();
        window.editing.cancel_all();
        let command = state
            .configuration
            .current()
            .commands
            .iter()
            .find(|c| c.id == "app.finder")
            .unwrap()
            .clone();
        window
            .editing
            .open(&state.configuration, &command, crate::actions::Mode::Input);
        let input = window.editing.token().unwrap();
        window
            .editing
            .update_input(input, 1, "PRIVATE typed value Ω")
            .unwrap();
        window
            .editing
            .edit_input(&mut state.configuration, crate::actions::Mode::Alias)
            .unwrap();
        let child = window.editing.token().unwrap();
        let _server = install_writer(&mut state);
        let deadline = Instant::now() + Duration::from_millis(200);
        let (entered, wait) = mpsc::channel();
        let (release, hold) = mpsc::channel();
        start_configuration_edit(
            &mut state,
            &mut window,
            &ui,
            deadline,
            move |store, editing, apps, _| {
                editing.update_draft(child, 1, Some("owned-child".into()), None);
                let receipt = editing.save_with_apps(store, child, None, None, apps)?;
                entered.send(()).unwrap();
                hold.recv().unwrap();
                Ok((receipt.warning, ConfigAfter::None))
            },
        )
        .unwrap();
        wait.recv_timeout(Duration::from_secs(2)).unwrap();
        std::thread::sleep(
            deadline.saturating_duration_since(Instant::now()) + Duration::from_millis(2),
        );
        release.send(()).unwrap();
        settle(&mut state, &mut window);
        let resumed = window.editing.view().unwrap();
        assert_eq!(resumed.resumed_input_from, Some(input));
        assert_eq!(resumed.command["input"], "PRIVATE typed value Ω");
        assert_ne!(resumed.token, input);
        assert_ne!(resumed.token, child);
    }
    #[test]
    fn query_change_preserves_owned_autosave_but_denies_stale_second_write() {
        let (_temp, mut state, mut window, ui, _) = setup();
        let _server = install_writer(&mut state);
        let token = window.editing.token().unwrap();
        let (entered, wait) = mpsc::channel();
        let (release, hold) = mpsc::channel();
        start_configuration_edit(
            &mut state,
            &mut window,
            &ui,
            Instant::now() + Duration::from_secs(2),
            move |store, editing, apps, followup_live| {
                entered.send(()).unwrap();
                hold.recv().unwrap();
                editing.update_draft(token, 1, Some("query-saved".into()), None);
                let receipt = editing.save_with_apps(store, token, None, None, apps)?;
                if followup_live() {
                    let second = store
                        .current()
                        .commands
                        .iter()
                        .find(|c| c.id == "app.safari")
                        .unwrap()
                        .clone();
                    crate::actions::set_enabled(store, &second, false)?;
                }
                Ok((receipt.warning, ConfigAfter::None))
            },
        )
        .unwrap();
        wait.recv_timeout(Duration::from_secs(2)).unwrap();
        ui.query("safari");
        process_inputs(&mut state, &mut window, &ui);
        release.send(()).unwrap();
        settle(&mut state, &mut window);
        assert_eq!(window.session.query, "safari");
        assert!(window.editing.token().is_none());
        assert_eq!(
            state
                .configuration
                .current()
                .commands
                .iter()
                .find(|c| c.id == "app.finder")
                .unwrap()
                .aliases,
            ["query-saved"]
        );
        assert_ne!(
            state
                .configuration
                .current()
                .commands
                .iter()
                .find(|c| c.id == "app.safari")
                .unwrap()
                .enabled,
            Some(false)
        );
    }
    #[test]
    fn direct_owner_replacement_revokes_prepublication_without_needing_ui_cancel() {
        let (_temp, mut state, mut window, ui, _) = setup();
        let _server = install_writer(&mut state);
        let before = state.configuration.snapshot().to_vec();
        let token = window.editing.token().unwrap();
        let (entered, wait) = mpsc::channel();
        let (release, hold) = mpsc::channel();
        start_configuration_edit(
            &mut state,
            &mut window,
            &ui,
            Instant::now() + Duration::from_secs(2),
            move |store, editing, apps, _| {
                entered.send(()).unwrap();
                hold.recv().unwrap();
                editing.update_draft(token, 1, Some("must-not-save".into()), None);
                let receipt = editing.save_with_apps(store, token, None, None, apps)?;
                Ok((receipt.warning, ConfigAfter::None))
            },
        )
        .unwrap();
        wait.recv_timeout(Duration::from_secs(2)).unwrap();
        window.editing.cancel_all();
        window
            .editing
            .create(&state.configuration, "url", None)
            .unwrap();
        let newer = window.editing.view().unwrap();
        assert!(!ui.editor_cancelled(token));
        release.send(()).unwrap();
        settle(&mut state, &mut window);
        assert_eq!(window.editing.view().unwrap(), newer);
        assert_eq!(std::fs::read(&state.configuration.file).unwrap(), before);
    }
    #[test]
    fn save_editor_uses_writer_and_adopts_saved_alias_without_reopening_form() {
        let (_temp, mut state, mut window, ui, _) = setup();
        let _server = install_writer(&mut state);
        let token = window.editing.token().unwrap();
        ui.save_editor(token, Some("fast-finder".into()), None, None);
        process_inputs(&mut state, &mut window, &ui);
        assert!(state.config_busy());
        assert!(window.editing.is_saving());
        settle(&mut state, &mut window);
        assert_eq!(
            state
                .configuration
                .current()
                .commands
                .iter()
                .find(|c| c.id == "app.finder")
                .unwrap()
                .aliases,
            ["fast-finder"]
        );
        assert!(window.editing.token().is_none());
    }
    #[test]
    fn save_behind_held_internal_watch_preserves_admission_cancel_and_expiry() {
        for outcome in ["save", "cancel", "expire"] {
            let (_temp, mut state, mut window, ui, _) = setup();
            let _server = install_writer(&mut state);
            let token = window.editing.token().unwrap();
            let before = std::fs::read(&state.configuration.file).unwrap();
            let (entered, entry) = mpsc::channel();
            let (release, released) = mpsc::channel();
            state
                .writer
                .as_mut()
                .unwrap()
                .start(
                    &state.configuration,
                    Arc::new(|| true),
                    false,
                    move |store| {
                        entered.send(()).unwrap();
                        released.recv().unwrap();
                        ConfigPayload::reload(store)
                    },
                )
                .unwrap();
            state.config_context = Some(ConfigContext::Watch);
            entry.recv_timeout(Duration::from_secs(2)).unwrap();
            ui.save_editor(token, Some("after-watch".into()), None, None);
            let intent = ui.latest_intent();
            process_inputs(&mut state, &mut window, &ui);
            assert!(window.deferred_save.is_some());
            assert!(window.editing.is_saving());
            assert_eq!(std::fs::read(&state.configuration.file).unwrap(), before);
            // No sleeps or eventual race: the watch is positively held now.
            if outcome == "cancel" {
                ui.action(Action::CancelEditor { token });
                process_inputs(&mut state, &mut window, &ui);
                assert!(window.deferred_save.is_none());
            } else if outcome == "expire" {
                window.deferred_save.as_mut().unwrap().deadline = Instant::now();
                process_inputs(&mut state, &mut window, &ui);
                assert!(window.deferred_save.is_none());
                assert!(!window.editing.is_saving());
                assert!(window.editing.view().unwrap().error.contains("expired"));
            }
            release.send(()).unwrap();
            settle(&mut state, &mut window);
            process_inputs(&mut state, &mut window, &ui);
            settle(&mut state, &mut window);
            assert!(window.deferred_save.is_none());
            if outcome == "save" {
                assert!(window.editing.token().is_none());
                assert_eq!(
                    ui.latest_intent(),
                    intent,
                    "replay must not mint another UI intent"
                );
                assert_eq!(
                    state
                        .configuration
                        .current()
                        .commands
                        .iter()
                        .find(|c| c.id == "app.finder")
                        .unwrap()
                        .aliases,
                    ["after-watch"]
                );
            } else {
                assert_eq!(std::fs::read(&state.configuration.file).unwrap(), before);
            }
        }
    }
    #[test]
    fn inline_transition_saves_previous_then_opens_distinct_followup_token() {
        let (_temp, mut state, mut window, ui, target) = setup();
        let _server = install_writer(&mut state);
        let token = window.editing.token().unwrap();
        ui.begin_inline(
            target,
            crate::actions::Mode::Alias,
            Some((token, Some("first-alias".into()), None)),
        );
        process_inputs(&mut state, &mut window, &ui);
        assert!(window.editing.is_saving());
        settle(&mut state, &mut window);
        assert_eq!(
            state
                .configuration
                .current()
                .commands
                .iter()
                .find(|c| c.id == "app.finder")
                .unwrap()
                .aliases,
            ["first-alias"]
        );
        let next = window.editing.view().unwrap();
        assert_ne!(next.token, token);
        assert_eq!(next.title, "Safari");
        assert!(!next.saving);
    }
    #[test]
    fn checkbox_after_inline_save_keeps_both_published_changes() {
        let (_temp, mut state, mut window, ui, target) = setup();
        let _server = install_writer(&mut state);
        let token = window.editing.token().unwrap();
        ui.set_enabled(
            target,
            true,
            false,
            Some((token, Some("finder-first".into()), None)),
        );
        process_inputs(&mut state, &mut window, &ui);
        settle(&mut state, &mut window);
        assert_eq!(
            state
                .configuration
                .current()
                .commands
                .iter()
                .find(|c| c.id == "app.finder")
                .unwrap()
                .aliases,
            ["finder-first"]
        );
        assert_eq!(
            state
                .configuration
                .current()
                .commands
                .iter()
                .find(|c| c.id == "app.safari")
                .unwrap()
                .enabled,
            Some(false)
        );
        assert!(window.editing.token().is_none());
    }
    #[test]
    fn published_late_completion_preserves_new_editor_and_never_reuses_worker_token() {
        let (_temp, mut state, mut window, ui, _) = setup();
        let _server = install_writer(&mut state);
        let old = window.editing.token().unwrap();
        let (entered, wait) = mpsc::channel();
        let (release, hold) = mpsc::channel();
        start_configuration_edit(
            &mut state,
            &mut window,
            &ui,
            Instant::now() + Duration::from_secs(3),
            move |store, editing, apps, _| {
                editing.update_draft(old, 1, Some("persisted".into()), None);
                let receipt = editing.save_with_apps(store, old, None, None, apps)?;
                editing.create(store, "snippet", None)?;
                entered.send(editing.token().unwrap()).unwrap();
                hold.recv().unwrap();
                Ok((receipt.warning, ConfigAfter::None))
            },
        )
        .unwrap();
        let discarded = wait.recv_timeout(Duration::from_secs(2)).unwrap();
        ui.action(Action::CancelEditor { token: old });
        process_inputs(&mut state, &mut window, &ui);
        window
            .editing
            .create(&state.configuration, "url", None)
            .unwrap();
        let newer = window.editing.view().unwrap();
        assert!(newer.token > discarded);
        release.send(()).unwrap();
        settle(&mut state, &mut window);
        assert_eq!(window.editing.view().unwrap(), newer);
        assert_eq!(
            state
                .configuration
                .current()
                .commands
                .iter()
                .find(|c| c.id == "app.finder")
                .unwrap()
                .aliases,
            ["persisted"]
        );
    }
    fn pending_creation() -> (
        Temp,
        SharedState,
        WindowRuntime,
        Arc<Connection>,
        crate::main_services::Server,
    ) {
        let (temp, mut state, mut window, ui, _) = setup();
        window.editing.cancel_all();
        let (client, server) = crate::main_services::channel();
        state.services = Some(client);
        perform_named(
            &mut state,
            &mut window,
            "new-quicklink",
            &ui,
            None,
            ui.latest_dismissal(),
            Instant::now() + Duration::from_secs(2),
        )
        .unwrap();
        finish_creation(&mut state, &mut window, &ui);
        assert!(window.creation_request.as_ref().unwrap().reply.is_some());
        (temp, state, window, ui, server)
    }
    #[test]
    fn creation_stop_revokes_queued_read_while_draft_permit_still_lives() {
        let (_temp, state, window, ui, server) = pending_creation();
        ui.request_stop();
        assert!(!ui.stopped());
        let r = window.creation_request.as_ref().unwrap();
        assert!(!r._permit.live().load(std::sync::atomic::Ordering::Acquire));
        assert!(!creation_live(r, &state, &window, &ui));
        server.drain_with_prefill(
            || panic!("wrong operation"),
            |_| panic!("not an execution"),
            || panic!("stopped creation must not read"),
        );
    }
    #[test]
    fn creation_retirement_revokes_queued_service_before_shutdown_waits() {
        let (_temp, _state, mut window, _ui, server) = pending_creation();
        window.retire_creation();
        assert!(window.editing.token().is_none());
        server.drain_with_prefill(
            || panic!("wrong operation"),
            |_| panic!("not execution"),
            || panic!("retired shutdown request must not read"),
        );
    }
    #[test]
    fn creation_unavailable_service_is_optional_within_live_admission() {
        let (_temp, mut state, mut window, ui, server) = pending_creation();
        let token = window.editing.token();
        drop(server);
        finish_creation(&mut state, &mut window, &ui);
        window.publish(&state, &ui);
        assert_eq!(window.editing.token(), token);
        assert!(window.creation_request.is_none());
        assert_eq!(ui.view().editor.unwrap().command["target"], "https://");
    }
    #[test]
    fn creation_reply_reloads_actual_config_before_draft_publication() {
        let (_temp, mut state, mut window, ui, server) = pending_creation();
        let mut value: Value = serde_json::from_slice(state.configuration.snapshot()).unwrap();
        value["theme"] = json!("light");
        std::fs::write(
            &state.configuration.file,
            serde_json::to_vec(&value).unwrap(),
        )
        .unwrap();
        server.drain_with_prefill(
            || panic!("wrong operation"),
            |_| panic!("not an execution"),
            || Ok(Some("https://example.invalid/old".into())),
        );
        finish_creation(&mut state, &mut window, &ui);
        assert!(window.creation_request.is_none());
        assert!(window.editing.token().is_none());
        assert_eq!(
            state.configuration.current().theme.as_deref(),
            Some("light")
        );
    }
    #[test]
    fn creation_prefill_cannot_save_or_execute_until_explicit_user_save() {
        let (_temp, mut state, mut window, ui, server) = pending_creation();
        let before = std::fs::read(&state.configuration.file).unwrap();
        let token = window.editing.token().unwrap();
        window.publish(&state, &ui);
        assert!(ui.view().editor.is_none());
        server.drain_with_prefill(
            || panic!("wrong operation"),
            |_| panic!("not an execution"),
            || Ok(Some("https://example.invalid/own".into())),
        );
        finish_creation(&mut state, &mut window, &ui);
        window.publish(&state, &ui);
        assert_eq!(window.editing.token(), Some(token));
        assert_eq!(
            ui.view().editor.unwrap().command["target"],
            "https://example.invalid/own"
        );
        assert_eq!(std::fs::read(&state.configuration.file).unwrap(), before);
        assert_eq!(state.execution.count(), 0);
    }
    #[test]
    fn creation_deadline_retires_authority_instead_of_opening_expired_form() {
        let (_temp, mut state, mut window, ui, server) = pending_creation();
        window.creation_request.as_mut().unwrap().deadline = Instant::now();
        finish_creation(&mut state, &mut window, &ui);
        assert!(window.editing.token().is_none());
        server.drain_with_prefill(
            || panic!("wrong operation"),
            |_| panic!("not an execution"),
            || panic!("retired deadline must not read"),
        );
    }
    #[test]
    fn newer_query_or_menu_restores_visibility_after_rejected_completion_effect() {
        for query in [false, true] {
            let (_temp, mut state, mut window, ui, _target) = setup();
            window.editing.cancel_all();
            window.session.visible = false;
            window.execution_effect = Some((ui.latest_intent(), true));
            if query {
                ui.query("safari");
            } else {
                ui.action(Action::ToggleActions);
            }
            assert!(!process_inputs(&mut state, &mut window, &ui));
            assert!(window.session.visible);
            assert!(window.execution_effect.is_none());
            if query {
                assert_eq!(window.session.query, "safari");
            } else {
                assert!(window.session.actions_open);
            }
        }
    }
    #[test]
    fn obsolete_inline_target_acknowledges_and_retains_typed_draft() {
        let (_temp, mut shared, mut w, ui, mut target) = setup();
        let token = w.editing.token().unwrap();
        target.catalog_revision = target.catalog_revision.wrapping_sub(1);
        let (_, attempt) = ui
            .begin_inline(
                target,
                crate::actions::Mode::Alias,
                Some((token, Some("retained".into()), None)),
            )
            .unwrap();
        let before = std::fs::read(&shared.configuration.file).unwrap();
        assert!(!process_inputs(&mut shared, &mut w, &ui));
        let d = w.editing.view().unwrap();
        assert_eq!(d.attempt, attempt);
        assert_eq!(d.aliases, "retained");
        assert!(!d.error.is_empty());
        assert_eq!(std::fs::read(&shared.configuration.file).unwrap(), before);
    }
    #[test]
    fn expired_inline_transition_acknowledges_without_losing_draft() {
        let (_temp, mut shared, mut w, ui, target) = setup();
        let token = w.editing.token().unwrap();
        let (_, attempt) = ui
            .begin_inline(
                target,
                crate::actions::Mode::Alias,
                Some((token, Some("retained".into()), None)),
            )
            .unwrap();
        ui.expire_test_transition();
        let before = std::fs::read(&shared.configuration.file).unwrap();
        process_inputs(&mut shared, &mut w, &ui);
        let d = w.editing.view().unwrap();
        assert_eq!(d.attempt, attempt);
        assert_eq!(d.aliases, "retained");
        assert!(d.error.contains("expired"));
        assert_eq!(std::fs::read(&shared.configuration.file).unwrap(), before);
    }
    #[test]
    fn retired_result_actions_cannot_change_restored_or_replacement_intent() {
        for replacement in [false, true] {
            for name in [
                "close", "details", "choices", "up", "down", "select", "choose", "activate",
                "reload",
            ] {
                let (_temp, mut state, mut window, ui, _target) = setup();
                window.editing.cancel_all();
                let command: rustcast_core::Command = serde_json::from_value(json!({
                    "id":"fixture","title":"Fixture","kind":"script","target":"/usr/bin/true"
                }))
                .unwrap();
                window
                    .results
                    .present(
                        &command,
                        &state.configuration.file,
                        crate::script_result::Interpreted {
                            response: json!({"ok":true}),
                            choices: None,
                            detail: Some("Old report".into()),
                            envelope: true,
                        },
                        None,
                    )
                    .unwrap();
                // Model a request accepted while the old view still existed,
                // then retired/replaced on the owner before queue drainage.
                ui.publish(crate::connected::View {
                    script_result: window.results.surface(),
                    ..Default::default()
                });
                if replacement {
                    window
                        .results
                        .present(
                            &command,
                            &state.configuration.file,
                            crate::script_result::Interpreted {
                                response: json!({"ok":true}),
                                choices: None,
                                detail: Some("Current report".into()),
                                envelope: true,
                            },
                            None,
                        )
                        .unwrap();
                } else {
                    window.results.back(1);
                }
                let before = window.results.surface();
                window.message = "Current warning".into();
                window.input_intent = 54;
                ui.action(Action::Result {
                    token: 1,
                    name: name.into(),
                    id: Some("retired".into()),
                    epoch: window.epoch,
                });
                assert!(!process_inputs(&mut state, &mut window, &ui));
                assert_eq!(
                    window.results.surface(),
                    before,
                    "{name} replacement={replacement}"
                );
                assert_eq!(
                    window.message, "Current warning",
                    "{name} replacement={replacement}"
                );
                assert_eq!(window.input_intent, 54, "{name} replacement={replacement}");
                assert_eq!(state.execution.count(), 0);
            }
        }
    }
}
