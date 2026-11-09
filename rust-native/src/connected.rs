//! Plain-data connection between the single runtime owner and the main-thread
//! view. No AppKit handles or second search/session model cross this boundary.
use crate::{control::HostEvent, presentation, session::Session};
use std::{
    collections::VecDeque,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc::SyncSender,
        Arc, Mutex,
    },
    time::{Duration, Instant},
};

#[derive(Clone, Debug, PartialEq)]
pub struct Row {
    pub id: String,
    pub icon: presentation::Icon,
    pub detail: Option<String>,
    pub keyword: Option<String>,
    pub title: String,
    pub kind: String,
    pub hotkey: Option<String>,
    pub imported_hotkey: Option<String>,
    pub action: &'static str,
    pub is_group: bool,
    pub aliases: Vec<String>,
    pub enabled: bool,
}
#[derive(Clone, Debug, PartialEq)]
pub struct PassiveNotice {
    pub serial: u64,
    pub epoch: u64,
    pub intent: u64,
    pub dismissal: u64,
    pub text: String,
    pub success: bool,
}
impl PassiveNotice {
    pub fn live(&self, ui: &Connection) -> bool {
        !ui.stop_requested()
            && self.epoch == ui.latest_epoch()
            && self.intent == ui.latest_intent()
            && self.dismissal == ui.latest_dismissal()
    }
}
#[derive(Clone, Debug, Default, PartialEq)]
pub struct View {
    pub launcher_scope: Option<crate::session::LauncherScope>,
    pub clipboard: Option<crate::clipboard_history::Surface>,
    pub monitor:Option<Arc<crate::monitor::Surface>>,
    pub process_sort:crate::processes::Sort,
    pub system_scope: Option<crate::system_search::Scope>,
    pub system_busy: bool,
    pub system_subtitles: std::collections::HashMap<String,String>,
    pub system_metadata: std::collections::HashMap<String,crate::system_search::Metadata>,
    pub utility_scope:Option<String>,
    pub utility_busy:bool,
    pub dashboard: Option<Arc<crate::analytics_dashboard::Surface>>,
    pub settings: Option<Arc<crate::settings_surface::Surface>>,
    pub menu_bar:Option<crate::menu_bar::State>,
    pub dashboard_request: Option<(u64, u64, u64)>,
    pub passive_notice: Option<PassiveNotice>,
    pub app_choices: crate::app_choice::Choices,
    pub app_choices_pending: bool,
    pub revision: u64,
    pub rows_revision: u64,
    pub catalog_revision: u64,
    pub dismissal: u64,
    pub epoch: u64,
    pub query: String,
    pub rows: Arc<Vec<Row>>,
    pub library: Option<crate::library::Info>,
    pub show_serial: u64,
    pub selection: Option<usize>,
    pub visible: bool,
    pub config_revision: u64,
    pub theme: Option<String>,
    pub message: String,
    pub actions_open: bool,
    pub menu_token: u64,
    pub action_items: Vec<crate::actions::Item>,
    pub action_selection: usize,
    pub action_target: Option<crate::actions::Target>,
    pub editor: Option<crate::actions::EditorView>,
    pub separate_command_keys: bool,
    pub execution_running: bool,
    pub completion_intent: Option<u64>,
    pub script_result: Option<crate::script_session::Surface>,
}
impl View {
    pub fn from_session(
        session: &Session,
        epoch: u64,
        config_revision: u64,
        theme: Option<String>,
        message: String,
    ) -> Self {
        let mut view = Self {
            launcher_scope: session.launcher_scope,
            clipboard: None,
            monitor:None,
            process_sort:Default::default(),
            system_scope: None,
            system_busy: false,
            system_subtitles: Default::default(),
            system_metadata: Default::default(),
            utility_scope:None,
            utility_busy:false,
            dashboard: None,
            settings:None,
            menu_bar:None,
            dashboard_request: None,
            passive_notice: None,
            app_choices: Default::default(),
            app_choices_pending: false,
            script_result: None,
            execution_running: false,
            completion_intent: None,
            revision: 0,
            rows_revision: 0,
            catalog_revision: session.results_revision(),
            dismissal: 0,
            epoch,
            query: session.query.clone(),
            rows: Arc::new(
                session
                    .rows()
                    .map(|c| Row {
                        id: c.id.clone(),
                        icon: presentation::icon(c),
                        detail: None,
                        keyword: c.keyword.clone(),
                        title: c.title.clone(),
                        kind: c.kind.clone(),
                        hotkey: c.hotkey.clone(),
                        imported_hotkey: c.imported_hotkey.clone(),
                        action: if session.library.is_some() {
                            "Edit"
                        } else {
                            presentation::action_title(c)
                        },
                        is_group: false,
                        aliases: c.aliases.clone(),
                        enabled: c.enabled != Some(false),
                    })
                    .collect(),
            ),
            library: session.library_info(),
            show_serial: 0,
            selection: session.selection,
            visible: session.visible,
            config_revision,
            theme,
            message,
            actions_open: false,
            menu_token: 0,
            action_items: vec![],
            action_selection: 0,
            action_target: None,
            editor: None,
            separate_command_keys: true,
        };
        if let Some(info) = &view.library {
            let commands = view.rows.clone();
            let group_by_id: std::collections::HashMap<_, _> = session
                .rows()
                .map(|c| (c.id.as_str(), crate::library::group(c)))
                .collect();
            let selected = session.selected().map(|c| &c.id);
            let mut rows = vec![];
            for group in &info.groups {
                rows.push(Row {
                    icon: presentation::Icon::Symbol(""),
                    detail: None,
                    keyword: None,
                    id: group.title.clone(),
                    title: format!(
                        "{} {} ({})",
                        if group.collapsed && session.query.is_empty() {
                            "▸"
                        } else {
                            "▾"
                        },
                        group.title,
                        group.count
                    ),
                    kind: String::new(),
                    hotkey: None,
                    imported_hotkey: None,
                    action: "",
                    is_group: true,
                    aliases: vec![],
                    enabled: true,
                });
                rows.extend(
                    commands
                        .iter()
                        .filter(|row| {
                            group_by_id
                                .get(row.id.as_str())
                                .is_some_and(|name| *name == group.title)
                        })
                        .cloned(),
                );
            }
            view.selection =
                selected.and_then(|id| rows.iter().position(|row| !row.is_group && &row.id == id));
            view.rows = Arc::new(rows);
        }
        view
    }
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum InputOperation {
    Run,
    Expand,
    Menu,
}
#[derive(Clone, Debug)]
pub enum Action {
    PreviewInput { token: u64, value: String },
    LauncherBack(crate::session::LauncherScope),
    Clipboard{operation:String,id:Option<String>,revision:u64,epoch:u64,dismissal:u64},
    Monitor{token:u64,operation:crate::monitor::Operation},
    ProcessSort(crate::processes::Sort),
    OpenDashboard,
    Dashboard { token: u64, operation: crate::analytics_dashboard::Operation },
    Setting { revision:u64, navigation_revision:u64, operation:crate::settings_surface::Operation },
    Result {
        token: u64,
        name: String,
        id: Option<String>,
        epoch: u64,
    },
    InputControl {
        token: u64,
        attempt: u64,
        value: String,
        operation: InputOperation,
    },
    InputMenu {
        token: u64,
        menu_token: u64,
        key: Option<String>,
        name: Option<String>,
    },
    Show,
    BeginInline {
        target: crate::actions::Target,
        mode: crate::actions::Mode,
        previous: Option<crate::actions::InlineSubmission>,
    },
    SetEnabled {
        target: crate::actions::Target,
        expected: bool,
        enabled: bool,
        previous: Option<crate::actions::InlineSubmission>,
    },
    InlineGlobal {
        name: String,
        previous: Option<crate::actions::InlineSubmission>,
    },
    Library {
        kind: String,
        value: String,
        catalog_revision: u64,
        previous: Option<crate::actions::InlineSubmission>,
    },
    ToggleActions,
    Named {
        name: String,
        target: Option<crate::actions::Target>,
        menu_token: Option<u64>,
    },
    MenuKey {
        key: String,
        target: Option<crate::actions::Target>,
        menu_token: u64,
    },
    SubmitTemplate {
        token: u64,
        attempt: u64,
        values: Vec<(String, String)>,
    },
    SubmitInput {
        token: u64,
        attempt: u64,
        value: String,
    },
    SaveEditor {
        token: u64,
        attempt: u64,
        aliases: Option<String>,
        hotkey: Option<Option<String>>,
        command: Option<serde_json::Value>,
    },
    CancelEditor {
        token: u64,
    },
    CloseActions {
        token: u64,
    },
    Key(String),
    Select {
        id: Option<String>,
        catalog_revision: u64,
        epoch: u64,
    },
    Activate {
        id: String,
        catalog_revision: u64,
        epoch: u64,
    },
}
pub struct Input {
    pub intent: u64,
    pub epoch: u64,
    pub action: Action,
    pub deadline: Instant,
    pub received: Instant,
    pub dismissal: u64,
}
pub type QueryInput = (u64, String, Instant, bool, Option<u64>);
#[derive(Default)]
struct Inputs {
    preview: Option<Input>,
    query: Option<QueryInput>,
    actions: VecDeque<Input>,
    overflow: bool,
    close: Option<Input>,
    surface_cancel: Option<Input>,
    save: Option<Input>,
    transition: Option<Input>,
}
type Wake = Arc<dyn Fn() + Send + Sync>;
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HideKey {
    pub trace: String,
    pub epoch: u64,
    pub dismissal: u64,
    pub intent: u64,
    pub serial: u64,
}
#[derive(Clone)]
pub struct HideToken {
    pub ui: std::sync::Weak<Connection>,
    pub key: HideKey,
}
pub struct Connection {
    open_hide: Mutex<Option<(HideKey, u8)>>,
    performance_open: Mutex<Option<(u64, crate::performance_trace::Span)>>,
    sender: SyncSender<HostEvent>,
    inputs: Mutex<Inputs>,
    epoch: AtomicU64,
    submitted: Mutex<Option<String>>,
    query_marked: AtomicBool,
    submitted_result: AtomicU64,
    result_cancelled: AtomicU64,
    input_control_cancelled: AtomicU64,
    dismissal: AtomicU64,
    editor_cancelled: AtomicU64,
    submit_serial: AtomicU64,
    routing_revision: AtomicU64,
    intent: AtomicU64,
    consent_selection_revision:AtomicU64,
    menu_cancelled: AtomicU64,
    view: Mutex<View>,
    palette_visible: AtomicBool,
    wake: Mutex<Option<Wake>>,
    applied: AtomicU64,
    local_hidden: Mutex<Option<Instant>>,
    stop: AtomicBool,
    stopped: AtomicBool,
    outcome: Mutex<Option<Result<serde_json::Value, String>>>,
    probes: Mutex<VecDeque<crate::control::Envelope>>,
    pub hidden_tests: bool,
    pub is_management: bool,
    peer: Mutex<Option<std::sync::Weak<Connection>>>,
    prefill_armed: AtomicBool,
    prefill_runs: AtomicU64,
}
impl Connection {
    pub fn new(sender: SyncSender<HostEvent>, hidden_tests: bool) -> Arc<Self> {
        Self::with_role(sender, hidden_tests, false)
    }
    pub fn new_management(sender: SyncSender<HostEvent>, hidden_tests: bool) -> Arc<Self> {
        Self::with_role(sender, hidden_tests, true)
    }
    fn with_role(
        sender: SyncSender<HostEvent>,
        hidden_tests: bool,
        is_management: bool,
    ) -> Arc<Self> {
        Arc::new(Self {
            open_hide: Mutex::new(None),
            performance_open: Mutex::new(None),
            sender,
            probes: Mutex::new(VecDeque::new()),
            hidden_tests,
            is_management,
            peer: Mutex::new(None),
            prefill_armed: AtomicBool::new(false),
            prefill_runs: AtomicU64::new(0),
            inputs: Mutex::new(Inputs::default()),
            epoch: AtomicU64::new(0),
            submitted: Mutex::new(None),
            query_marked: AtomicBool::new(false),
            submitted_result: AtomicU64::new(0),
            result_cancelled: AtomicU64::new(0),
            input_control_cancelled: AtomicU64::new(0),
            dismissal: AtomicU64::new(0),
            editor_cancelled: AtomicU64::new(0),
            submit_serial: AtomicU64::new(0),
            routing_revision: AtomicU64::new(0),
            intent: AtomicU64::new(0),
            consent_selection_revision:AtomicU64::new(0),
            menu_cancelled: AtomicU64::new(0),
            palette_visible: AtomicBool::new(false),
            view: Mutex::new(View::default()),
            wake: Mutex::new(None),
            applied: AtomicU64::new(0),
            local_hidden: Mutex::new(None),
            stop: AtomicBool::new(false),
            stopped: AtomicBool::new(false),
            outcome: Mutex::new(None),
        })
    }
    pub fn begin_open_trace(&self, serial: u64) {
        let mut pending = self.performance_open.lock().unwrap_or_else(|p| p.into_inner());
        if self.stop_requested() || self.stopped() { return; }
        let span = crate::performance_trace::shared().begin(crate::performance_trace::Stage::OpenReady);
        let previous = pending.replace((serial, span));
        drop(pending);
        if let Some((_, previous)) = previous { crate::performance_trace::shared().finish(previous, true); }
    }
    pub fn finish_open_trace(&self, serial: u64, ready: bool) {
        let mut pending = self.performance_open.lock().unwrap_or_else(|p| p.into_inner());
        let span = if pending.as_ref().is_some_and(|(current, _)| *current == serial) {
            pending.take().map(|(_, span)| span)
        } else { None };
        drop(pending);
        if let Some(span) = span { crate::performance_trace::shared().finish(span, !ready); }
    }
    pub fn cancel_open_trace(&self) {
        let span = self.performance_open.lock().unwrap_or_else(|p| p.into_inner()).take();
        if let Some((_, span)) = span { crate::performance_trace::shared().finish(span, true); }
    }
    pub fn forward_probe(&self, request: crate::control::Envelope) {
        let mut probes = self.probes.lock().unwrap_or_else(|p| p.into_inner());
        probes.retain(|p| p.context.is_live());
        if probes.len() >= 16 {
            drop(probes);
            let _ =
                request.respond(serde_json::json!({"ok":false,"error":"Native probe is busy."}));
            return;
        }
        probes.push_back(request);
        drop(probes);
        self.signal();
    }
    pub fn take_probes(&self) -> Vec<crate::control::Envelope> {
        let mut probes = self.probes.lock().unwrap_or_else(|p| p.into_inner());
        let count = probes.len().min(8);
        let batch = probes.drain(..count).collect();
        let more = !probes.is_empty();
        drop(probes);
        if more {
            self.signal();
        }
        batch
    }
    pub fn link_windows(primary: &Arc<Self>, management: &Arc<Self>) {
        *primary.peer.lock().unwrap_or_else(|p| p.into_inner()) = Some(Arc::downgrade(management));
        *management.peer.lock().unwrap_or_else(|p| p.into_inner()) = Some(Arc::downgrade(primary));
    }
    pub fn peer(&self) -> Option<Arc<Self>> {
        self.peer
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .as_ref()
            .and_then(std::sync::Weak::upgrade)
    }
    pub fn adopt_query(&self, query: &str, epoch: u64) {
        let mut submitted = self.submitted.lock().unwrap_or_else(|p| p.into_inner());
        if epoch == self.latest_epoch() {
            *submitted = Some(query.into());
        }
    }
    pub fn view_summary(&self) -> (u64, usize) {
        let view = self.view.lock().unwrap_or_else(|p| p.into_inner());
        (view.revision, view.rows.len())
    }
    pub fn routing_revision(&self) -> u64 {
        self.routing_revision.load(Ordering::Acquire)
    }
    pub fn global_action(&self) -> u64 {
        self.intent.fetch_add(1, Ordering::AcqRel) + 1
    }
    pub fn latest_intent(&self) -> u64 {
        self.intent.load(Ordering::Acquire)
    }
    pub fn consent_selection_revision(&self)->u64{self.consent_selection_revision.load(Ordering::Acquire)}
    /// The runtime calls this only after accepting ordinary palette selection.
    /// It preserves consent refresh selection without changing action authority.
    pub fn note_palette_selection(&self){self.consent_selection_revision.fetch_add(1,Ordering::AcqRel);}
    fn hide_clock_matches(&self, key: &HideKey) -> bool {
        !self.is_management
            && !self.stop_requested()
            && self.latest_epoch() == key.epoch
            && self.latest_dismissal() == key.dismissal
            && self.latest_intent() == key.intent
    }
    pub fn arm_open_hide(self: &Arc<Self>, key: HideKey) -> Result<HideToken, String> {
        let mut slot = self.open_hide.lock().unwrap_or_else(|p| p.into_inner());
        if !self.hide_clock_matches(&key)
            || slot
                .as_ref()
                .is_some_and(|(old, _)| old.serial >= key.serial)
        {
            return Err("Window intent changed before opening.".into());
        }
        *slot = Some((key.clone(), 0));
        Ok(HideToken {
            ui: Arc::downgrade(self),
            key,
        })
    }
    pub fn cancel_open_hide(&self) {
        *self.open_hide.lock().unwrap_or_else(|p| p.into_inner()) = None;
    }
    pub fn open_hide_live(&self, key: &HideKey) -> bool {
        self.hide_clock_matches(key)
            && self
                .open_hide
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .as_ref()
                .is_some_and(|(k, _)| k == key)
    }
    pub fn open_hide_masks(&self, epoch: u64) -> bool {
        self.open_hide
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .as_ref()
            .is_some_and(|(k, ack)| *ack == 2 && k.epoch == epoch && self.hide_clock_matches(k))
    }
    /// Main calls the closure on its own controller. Ack is specific to the
    /// armed execution and only follows actual orderOut/visibility readback.
    pub fn finish_open_hide(&self, key: &HideKey) {
        let mut slot = self.open_hide.lock().unwrap_or_else(|p| p.into_inner());
        if slot.as_ref().is_some_and(|(k, _)| k == key) {
            *slot = None;
        }
    }
    pub fn acknowledge_open_hide(
        &self,
        key: &HideKey,
        hide: impl FnOnce() -> bool,
    ) -> Result<HideKey, String> {
        {
            let mut slot = self.open_hide.lock().unwrap_or_else(|p| p.into_inner());
            if !self.hide_clock_matches(key)
                || !slot
                    .as_ref()
                    .is_some_and(|(k, phase)| k == key && *phase == 0)
            {
                return Err("Window hide request is stale or already acknowledged.".into());
            }
            *slot = Some((key.clone(), 1)); // Claim once; no shared owner lock across AppKit.
        }
        let hidden = hide();
        let mut slot = self.open_hide.lock().unwrap_or_else(|p| p.into_inner());
        if !hidden
            || !self.hide_clock_matches(key)
            || !slot
                .as_ref()
                .is_some_and(|(k, phase)| k == key && *phase == 1)
        {
            return Err("Window could not be hidden for this execution.".into());
        }
        *slot = Some((key.clone(), 2));
        Ok(key.clone())
    }
    pub fn latest_epoch(&self) -> u64 {
        self.epoch.load(Ordering::Acquire)
    }
    pub fn attach_wake(&self, wake: Wake) {
        *self.wake.lock().unwrap_or_else(|p| p.into_inner()) = Some(wake);
        self.signal();
    }
    fn signal(&self) {
        let wake = self.wake.lock().unwrap_or_else(|p| p.into_inner()).clone();
        if let Some(wake) = wake {
            wake();
        }
    }
    fn notify(&self) {
        let _ = self.sender.try_send(HostEvent::UiWake);
    }
    pub fn query(&self, query: &str) {
        self.query_composing(query, false);
    }
    pub fn result_cancelled(&self, token: u64) -> bool {
        self.result_cancelled.load(Ordering::Acquire) >= token
    }
    pub fn cancel_result(&self, token: u64) {
        self.result_cancelled.fetch_max(token, Ordering::AcqRel);
        self.notify();
    }
    pub fn query_result(&self, token: u64, query: &str, marked: bool) {
        self.query_owned(query, marked, Some(token));
    }
    pub fn query_composing(&self, query: &str, marked: bool) {
        self.query_owned(query, marked, None);
    }
    fn query_owned(&self, query: &str, marked: bool, token: Option<u64>) {
        // Same lock order as publication: view -> submitted -> inputs. Retired
        // result traffic must not alter even the ordinary query/intent clocks.
        let result_view = token.map(|_| self.view.lock().unwrap_or_else(|p| p.into_inner()));
        if token.is_some_and(|token| {
            self.result_cancelled(token)
                || result_view
                    .as_ref()
                    .and_then(|v| v.script_result.as_ref())
                    .map(|r| r.token)
                    != Some(token)
        }) {
            return;
        }
        let mut submitted = self.submitted.lock().unwrap_or_else(|p| p.into_inner());
        if submitted.as_deref() == Some(query)
            && self.submitted_result.load(Ordering::Acquire) == token.unwrap_or(0)
            && self.query_marked.load(Ordering::Acquire) == marked
        {
            return;
        }
        *submitted = Some(query.into());
        self.submitted_result
            .store(token.unwrap_or(0), Ordering::Release);
        self.query_marked.store(marked, Ordering::Release);
        self.routing_revision.fetch_add(1, Ordering::AcqRel);
        self.intent.fetch_add(1, Ordering::AcqRel);
        let epoch = self.epoch.fetch_add(1, Ordering::AcqRel) + 1;
        self.inputs.lock().unwrap_or_else(|p| p.into_inner()).query =
            Some((epoch, query.into(), Instant::now(), marked, token));
        drop(submitted);
        drop(result_view);
        self.notify();
    }
    pub fn dashboard_action(&self, token:u64, operation:crate::analytics_dashboard::Operation) {
        let view=self.view.lock().unwrap_or_else(|e|e.into_inner());
        if !self.is_management || !view.visible || self.stop_requested()
            || view.dismissal!=self.latest_dismissal()
            || view.dashboard.as_ref().is_none_or(|s|s.token!=token) { return; }
        self.action(Action::Dashboard {token,operation});
    }
    pub fn monitor_action(&self,token:u64,operation:crate::monitor::Operation){
        let view=self.view.lock().unwrap_or_else(|p|p.into_inner());
        if self.stop_requested() || !view.monitor.as_ref().is_some_and(|m|m.token==token&&m.shown){return;}
        self.action(Action::Monitor{token,operation});
    }
    pub fn action(&self, action: Action) {
        let result_view = if let Action::Result { token, .. } = &action {
            let view = self.view.lock().unwrap_or_else(|p| p.into_inner());
            if self.result_cancelled(*token)
                || view.script_result.as_ref().map(|r| r.token) != Some(*token)
            {
                return;
            }
            Some(view)
        } else {
            None
        };
        let mut inputs = self.inputs.lock().unwrap_or_else(|p| p.into_inner());
        let closing = matches!(&action,Action::Key(key) if key=="cancelOperation:");
        let surface_cancel = matches!(
            &action,
            Action::CancelEditor { .. } | Action::CloseActions { .. }
        ) || matches!(&action,Action::Result{name,..} if name=="close");
        match &action {
            Action::Result { token, name, .. } if name == "close" => {
                self.result_cancelled.fetch_max(*token, Ordering::AcqRel);
            }
            Action::CancelEditor { token } => {
                self.editor_cancelled.fetch_max(*token, Ordering::AcqRel);
            }
            Action::CloseActions { token } => {
                self.menu_cancelled.fetch_max(*token, Ordering::AcqRel);
            }
            _ => {}
        }
        let dismissal = if closing {
            self.dismissal.fetch_add(1, Ordering::AcqRel) + 1
        } else {
            self.dismissal.load(Ordering::Acquire)
        };
        self.routing_revision.fetch_add(1, Ordering::AcqRel);
        let input = Input {
            intent: self.intent.fetch_add(1, Ordering::AcqRel) + 1,
            epoch: self.epoch.load(Ordering::Acquire),
            action,
            deadline: Instant::now() + Duration::from_secs(5),
            received: Instant::now(),
            dismissal,
        };
        if closing {
            inputs.close = Some(input);
        } else if surface_cancel {
            inputs.surface_cancel = Some(input);
        } else if matches!(
            &input.action,
            Action::BeginInline { .. }
                | Action::InlineGlobal { .. }
                | Action::Library {
                    previous: Some(_),
                    ..
                }
                | Action::SetEnabled {
                    previous: Some(_),
                    ..
                }
        ) {
            inputs.transition = Some(input);
        } else if matches!(
            &input.action,
            Action::SaveEditor { .. }
                | Action::SubmitTemplate { .. }
                | Action::SubmitInput { .. }
                | Action::InputControl { .. }
        ) {
            inputs.save = Some(input);
        } else if matches!(&input.action, Action::PreviewInput { .. }) {
            inputs.preview = Some(input);
        } else if inputs.actions.len() < 128 {
            inputs.actions.push_back(input);
        } else {
            inputs.overflow = true;
        }
        drop(inputs);
        drop(result_view);
        self.notify();
        if closing || surface_cancel {
            self.signal();
        }
    }
    pub fn cancel_input_control(&self, attempt: u64) {
        self.input_control_cancelled
            .fetch_max(attempt, Ordering::AcqRel);
        self.notify();
        self.signal();
    }
    pub fn input_control_cancelled(&self, attempt: u64) -> bool {
        attempt != 0 && attempt <= self.input_control_cancelled.load(Ordering::Acquire)
    }
    pub fn input_control(&self, token: u64, value: String, operation: InputOperation) -> u64 {
        let attempt = self.submit_serial.fetch_add(1, Ordering::AcqRel) + 1;
        self.action(Action::InputControl {
            token,
            attempt,
            value,
            operation,
        });
        attempt
    }
    pub fn submit_template(&self, token: u64, values: Vec<(String, String)>) -> u64 {
        let attempt = self.submit_serial.fetch_add(1, Ordering::AcqRel) + 1;
        self.action(Action::SubmitTemplate {
            token,
            attempt,
            values,
        });
        attempt
    }
    pub fn submit_input(&self, token: u64, value: String) -> u64 {
        let attempt = self.submit_serial.fetch_add(1, Ordering::AcqRel) + 1;
        self.action(Action::SubmitInput {
            token,
            attempt,
            value,
        });
        attempt
    }
    #[cfg(feature = "test-tools")]
    pub fn expire_input_for_test(&self) -> bool {
        if !self.hidden_tests {
            return false;
        }
        let mut inputs = self.inputs.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(input) = inputs.save.as_mut().filter(|i| {
            matches!(
                i.action,
                Action::SubmitTemplate { .. }
                    | Action::SubmitInput { .. }
                    | Action::InputControl { .. }
            )
        }) {
            input.deadline = Instant::now() - Duration::from_secs(1);
            true
        } else {
            false
        }
    }
    pub fn save_editor(
        &self,
        token: u64,
        aliases: Option<String>,
        hotkey: Option<Option<String>>,
        command: Option<serde_json::Value>,
    ) -> u64 {
        let attempt = self.submit_serial.fetch_add(1, Ordering::AcqRel) + 1;
        self.action(Action::SaveEditor {
            token,
            attempt,
            aliases,
            hotkey,
            command,
        });
        attempt
    }
    pub fn inline_global(
        &self,
        name: String,
        previous: Option<(u64, Option<String>, Option<Option<String>>)>,
    ) -> Option<(u64, u64)> {
        let previous = self.inline_submission(previous);
        let request = previous.as_ref().map(|p| (p.token, p.attempt));
        self.action(Action::InlineGlobal { name, previous });
        request
    }
    pub fn library_transition(
        &self,
        kind: String,
        value: String,
        catalog_revision: u64,
        previous: Option<(u64, Option<String>, Option<Option<String>>)>,
    ) -> Option<(u64, u64)> {
        let previous = self.inline_submission(previous);
        let request = previous.as_ref().map(|p| (p.token, p.attempt));
        self.action(Action::Library {
            kind,
            value,
            catalog_revision,
            previous,
        });
        request
    }
    pub fn set_enabled(
        &self,
        target: crate::actions::Target,
        expected: bool,
        enabled: bool,
        previous: Option<(u64, Option<String>, Option<Option<String>>)>,
    ) -> Option<(u64, u64)> {
        let previous = self.inline_submission(previous);
        let request = previous.as_ref().map(|p| (p.token, p.attempt));
        self.action(Action::SetEnabled {
            target,
            expected,
            enabled,
            previous,
        });
        request
    }
    fn inline_submission(
        &self,
        previous: Option<(u64, Option<String>, Option<Option<String>>)>,
    ) -> Option<crate::actions::InlineSubmission> {
        previous.map(
            |(token, aliases, hotkey)| crate::actions::InlineSubmission {
                token,
                attempt: self.submit_serial.fetch_add(1, Ordering::AcqRel) + 1,
                aliases,
                hotkey,
            },
        )
    }
    pub fn begin_inline(
        &self,
        target: crate::actions::Target,
        mode: crate::actions::Mode,
        previous: Option<(u64, Option<String>, Option<Option<String>>)>,
    ) -> Option<(u64, u64)> {
        let previous = self.inline_submission(previous);
        let request = previous.as_ref().map(|p| (p.token, p.attempt));
        self.action(Action::BeginInline {
            target,
            mode,
            previous,
        });
        request
    }
    #[cfg(test)]
    pub(crate) fn expire_test_transition(&self) {
        if let Some(input) = self.inputs.lock().unwrap().transition.as_mut() {
            input.deadline = Instant::now() - Duration::from_secs(1);
        }
    }
    pub fn arm_test_prefill(&self) -> bool {
        if cfg!(feature = "test-tools") && self.hidden_tests {
            self.prefill_armed.store(true, Ordering::Release);
            true
        } else {
            false
        }
    }
    pub fn test_prefill_runs(&self) -> u64 {
        self.prefill_runs.load(Ordering::Acquire)
    }
    pub fn take_test_prefill(&self) -> bool {
        if cfg!(feature = "test-tools")
            && self.hidden_tests
            && self.prefill_armed.swap(false, Ordering::AcqRel)
        {
            self.prefill_runs.fetch_add(1, Ordering::AcqRel);
            true
        } else {
            false
        }
    }
    pub fn editor_cancelled(&self, token: u64) -> bool {
        token != 0 && token <= self.editor_cancelled.load(Ordering::Acquire)
    }
    pub fn menu_cancelled(&self, token: u64) -> bool {
        token != 0 && token <= self.menu_cancelled.load(Ordering::Acquire)
    }
    pub fn latest_dismissal(&self) -> u64 {
        self.dismissal.load(Ordering::Acquire)
    }
    // The owner drains on every received event, so a full notification channel
    // cannot strand a coalesced query/action batch. Control requests remain bounded.
    pub fn drain(&self) -> (Option<QueryInput>, Vec<Input>, bool) {
        let mut input = self.inputs.lock().unwrap_or_else(|p| p.into_inner());
        let mut actions = Vec::with_capacity(input.actions.len() + 1);
        if let Some(close) = input.close.take() {
            actions.push(close);
        }
        if let Some(cancel) = input.surface_cancel.take() {
            actions.push(cancel);
        }
        if let Some(transition) = input.transition.take() {
            actions.push(transition);
        }
        if let Some(preview) = input.preview.take() {
            actions.push(preview);
        }
        if let Some(save) = input.save.take() {
            actions.push(save);
        }
        actions.extend(input.actions.drain(..));
        (
            input.query.take(),
            actions,
            std::mem::take(&mut input.overflow),
        )
    }
    pub fn publish(&self, mut next: View) {
        let mut view = self.view.lock().unwrap_or_else(|p| p.into_inner());
        let mut submitted = self.submitted.lock().unwrap_or_else(|p| p.into_inner());
        if next.epoch == self.latest_epoch()
            && next
                .script_result
                .as_ref()
                .is_none_or(|r| r.query.is_some())
        {
            *submitted = Some(next.query.clone());
            self.submitted_result.store(
                next.script_result.as_ref().map_or(0, |r| r.token),
                Ordering::Release,
            );
        }
        drop(submitted);
        next.revision = view.revision;
        next.rows_revision = view.rows_revision;
        if next.rows != view.rows
            || next.system_subtitles != view.system_subtitles
            || next.system_metadata != view.system_metadata
        {
            next.rows_revision = next.rows_revision.wrapping_add(1);
        }
        if next == *view {
            return;
        }
        next.revision = next.revision.wrapping_add(1);
        self.palette_visible.store(next.visible, Ordering::Release);
        *view = next;
        drop(view);
        self.signal();
    }
    /// Scalar projection keeps the tap off the worker's presentation mutex.
    pub fn palette_visible(&self) -> bool {
        self.palette_visible.load(Ordering::Acquire)
    }

    pub fn view(&self) -> View {
        self.view.lock().unwrap_or_else(|p| p.into_inner()).clone()
    }
    pub fn applied(&self, revision: u64) {
        self.applied.store(revision, Ordering::Release);
    }
    pub fn applied_revision(&self) -> u64 {
        self.applied.load(Ordering::Acquire)
    }
    /// A readonly owner observation. This does not grant endpoint admission and
    /// does not drain, cancel, or manufacture acknowledgements for queued work.
    #[cfg(feature = "test-tools")]
    pub fn pending_owned_inputs(&self) -> serde_json::Value {
        let inputs = self.inputs.lock().unwrap_or_else(|p| p.into_inner());
        let probes = self.probes.lock().unwrap_or_else(|p| p.into_inner());
        serde_json::json!({
            "query":usize::from(inputs.query.is_some()), "actions":inputs.actions.len(),
            "close":usize::from(inputs.close.is_some()),
            "surface_cancel":usize::from(inputs.surface_cancel.is_some()),
            "save":usize::from(inputs.save.is_some()),
            "preview":usize::from(inputs.preview.is_some()),
            "transition":usize::from(inputs.transition.is_some()),
            "overflow":inputs.overflow,"native_probes":probes.len()
        })
    }
    pub fn note_local_hide(&self) {
        *self.local_hidden.lock().unwrap_or_else(|p| p.into_inner()) = Some(Instant::now());
    }
    pub fn hidden_during(&self, start: Instant, end: Instant) -> bool {
        self.local_hidden
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .is_some_and(|at| at >= start && at <= end)
    }
    pub fn request_stop(&self) {
        self.stop.store(true, Ordering::Release);
        self.cancel_open_trace();
        self.notify();
    }
    pub fn stop_requested(&self) -> bool {
        self.stop.load(Ordering::Acquire)
    }
    pub fn finish(&self, result: Result<serde_json::Value, String>) {
        *self.outcome.lock().unwrap_or_else(|p| p.into_inner()) = Some(result);
        self.stopped.store(true, Ordering::Release);
        self.cancel_open_trace();
        self.signal();
    }
    pub fn stopped(&self) -> bool {
        self.stopped.load(Ordering::Acquire)
    }
    pub fn take_outcome(&self) -> Option<Result<serde_json::Value, String>> {
        self.outcome
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .take()
    }
}

impl Drop for Connection {
    fn drop(&mut self) {
        if let Some((_, span)) = self.performance_open.get_mut().unwrap_or_else(|p| p.into_inner()).take() {
            crate::performance_trace::shared().finish(span, true);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn open_trace_late_serial_cannot_finish_replacement_and_stop_retires_it() {
        let (sender, _) = std::sync::mpsc::sync_channel(4);
        let ui = Connection::new(sender, true);
        ui.begin_open_trace(1);
        ui.begin_open_trace(2);
        ui.finish_open_trace(1, true);
        assert_eq!(ui.performance_open.lock().unwrap().as_ref().map(|(serial, _)| *serial), Some(2));
        ui.finish_open_trace(2, false);
        assert!(ui.performance_open.lock().unwrap().is_none());
        ui.begin_open_trace(3);
        ui.request_stop();
        assert!(ui.performance_open.lock().unwrap().is_none());
        ui.begin_open_trace(4);
        ui.finish_open_trace(3, true);
        assert!(ui.performance_open.lock().unwrap().is_none());
        let (sender, _) = std::sync::mpsc::sync_channel(4);
        let finished = Connection::new(sender, true);
        finished.begin_open_trace(1);
        finished.finish(Ok(serde_json::json!({"ok":true})));
        finished.begin_open_trace(2);
        assert!(finished.performance_open.lock().unwrap().is_none());
    }
    #[cfg(feature = "test-tools")]
    #[test]
    fn owned_inspection_observes_without_draining_or_acknowledging() {
        let (tx, _rx) = std::sync::mpsc::sync_channel(32);
        let ui=Connection::new(tx,false);
        ui.query("owned pending query");
        let epoch=ui.latest_epoch();
        let before=ui.view();
        let observed=ui.pending_owned_inputs();
        assert_eq!(observed["query"],1);
        assert_eq!(ui.pending_owned_inputs(),observed);
        assert_eq!(ui.view(),before);
        assert_eq!(ui.applied_revision(),0);
        let (query,actions,overflow)=ui.drain();
        let query=query.expect("Inspection must retain the actual query.");
        assert_eq!(query.0,epoch);
        assert_eq!(query.1,"owned pending query");
        assert!(actions.is_empty());
        assert!(!overflow);
        assert_eq!(ui.pending_owned_inputs()["query"],0);
    }
    #[test]
    fn passive_notice_cannot_be_replayed_by_a_fresh_view_after_new_local_intent() {
        let (tx, _rx) = std::sync::mpsc::sync_channel(32);
        let ui = Connection::new(tx, true);
        let notice = PassiveNotice {
            serial: 1,
            epoch: 0,
            intent: 0,
            dismissal: 0,
            text: "Done".into(),
            success: true,
        };
        assert!(notice.live(&ui));
        let mut later = View {
            passive_notice: Some(notice),
            ..View::default()
        };
        ui.query("new query");
        later.epoch = ui.latest_epoch();
        assert!(!later.passive_notice.as_ref().unwrap().live(&ui));
    }
    #[test]
    fn hide_does_not_hold_owner_lock_and_revalidates_after_appkit() {
        let (tx, _rx) = std::sync::mpsc::sync_channel(32);
        let ui = Connection::new(tx, true);
        let key = HideKey {
            trace: "fixture".into(),
            epoch: 0,
            dismissal: 0,
            intent: 0,
            serial: 1,
        };
        ui.arm_open_hide(key.clone()).unwrap();
        assert!(ui
            .acknowledge_open_hide(&key, || {
                ui.cancel_open_hide();
                true
            })
            .is_err());
        assert!(!ui.open_hide_masks(0));
    }
    #[test]
    fn exact_open_hide_rejects_wrong_replayed_and_newer_intent_tokens() {
        let (tx, _rx) = std::sync::mpsc::sync_channel(32);
        let ui = Connection::new(tx, true);
        let key = HideKey {
            trace: "fixture".into(),
            epoch: 0,
            dismissal: 0,
            intent: 0,
            serial: 1,
        };
        let token = ui.arm_open_hide(key.clone()).unwrap();
        let mut wrong = key.clone();
        wrong.trace = "wrong".into();
        assert!(ui
            .acknowledge_open_hide(&wrong, || panic!("Wrong token must not hide"))
            .is_err());
        assert_eq!(ui.acknowledge_open_hide(&token.key, || true).unwrap(), key);
        assert!(ui.open_hide_masks(0));
        assert!(ui
            .acknowledge_open_hide(&key, || panic!("Replay must not hide"))
            .is_err());
        ui.query("new query");
        assert!(!ui.open_hide_live(&key));
        assert!(!ui.open_hide_masks(0));
        assert!(ui
            .acknowledge_open_hide(&key, || panic!("Old intent must not hide"))
            .is_err());
    }
    #[test]
    fn open_hide_requires_actual_hidden_readback_and_explicit_show_revokes() {
        let (tx, _rx) = std::sync::mpsc::sync_channel(32);
        let ui = Connection::new(tx, true);
        let key = HideKey {
            trace: "fixture".into(),
            epoch: 0,
            dismissal: 0,
            intent: 0,
            serial: 1,
        };
        ui.arm_open_hide(key.clone()).unwrap();
        assert!(ui.acknowledge_open_hide(&key, || false).is_err());
        assert!(!ui.open_hide_masks(0));
        ui.cancel_open_hide();
        assert!(ui
            .acknowledge_open_hide(&key, || panic!("New show must revoke old hide"))
            .is_err());
    }
    #[test]
    fn composition_commit_same_text_and_reserved_input_control_are_not_lost() {
        let (tx, _rx) = std::sync::mpsc::sync_channel(1);
        let ui = Connection::new(tx, false);
        ui.query_composing("alias 日本語", true);
        let first = ui.drain().0.unwrap();
        assert!(first.3);
        ui.query_composing("alias 日本語", false);
        let committed = ui.drain().0.unwrap();
        assert!(!committed.3);
        assert!(committed.0 > first.0);
        for _ in 0..128 {
            ui.action(Action::Key("moveDown:".into()));
        }
        let attempt = ui.input_control(7, "literal Ω".into(), InputOperation::Expand);
        ui.cancel_input_control(attempt);
        let (_, inputs, _) = ui.drain();
        assert!(inputs.iter().any(|i|matches!(&i.action,Action::InputControl{token:7,attempt:a,value,operation:InputOperation::Expand} if *a==attempt && value=="literal Ω")));
        assert!(ui.input_control_cancelled(attempt));
        assert!(!ui.input_control_cancelled(attempt + 1));
    }
    #[test]
    fn date_previews_coalesce_without_losing_submit_or_cancel() {
        let (tx, _rx) = std::sync::mpsc::sync_channel(1);
        let ui = Connection::new(tx, false);
        let epoch = ui.latest_epoch();
        for i in 0..1000 {
            ui.action(Action::PreviewInput { token: 7, value: i.to_string() });
        }
        ui.submit_input(7, "999".into());
        ui.action(Action::CancelEditor { token: 7 });
        let (query, actions, overflow) = ui.drain();
        assert!(query.is_none());
        assert!(!overflow);
        assert_eq!(ui.latest_epoch(), epoch);
        assert_eq!(actions.len(), 3);
        assert!(matches!(actions[0].action, Action::CancelEditor { token: 7 }));
        assert!(matches!(&actions[1].action, Action::PreviewInput { token: 7, value } if value == "999"));
        assert!(matches!(&actions[2].action, Action::SubmitInput { token: 7, value, .. } if value == "999"));
        assert!(ui.editor_cancelled(7));
    }
    #[test]
    fn saturated_notifications_keep_latest_query_rapid_keys_and_close() {
        let (tx, _rx) = std::sync::mpsc::sync_channel(1);
        let ui = Connection::new(tx, false);
        ui.query("notes");
        for _ in 0..5 {
            ui.query("notes");
            ui.action(Action::Key("moveDown:".into()));
        }
        let (query, keys, overflow) = ui.drain();
        assert_eq!(query.unwrap().0, 1);
        assert_eq!(keys.len(), 5);
        assert!(!overflow);
        assert!(keys.iter().all(|input| input.epoch == 1));
        for _ in 0..140 {
            ui.action(Action::Key("moveDown:".into()));
        }
        ui.action(Action::Key("cancelOperation:".into()));
        ui.query("newest");
        ui.query("latest");
        let (query, keys, overflow) = ui.drain();
        assert_eq!(query.unwrap().1, "latest");
        assert!(overflow);
        assert_eq!(keys.len(), 129);
        assert!(matches!(&keys[0].action,Action::Key(key) if key=="cancelOperation:"));
        assert_eq!(ui.latest_dismissal(), 1);
    }
    #[test]
    fn compound_transition_has_bounded_acknowledged_capacity_under_saturation() {
        let (tx, _rx) = std::sync::mpsc::sync_channel(1);
        let ui = Connection::new_management(tx, true);
        for _ in 0..140 {
            ui.action(Action::Key("moveDown:".into()));
        }
        let target = crate::actions::Target {
            id: "app.safari".into(),
            catalog_revision: 1,
            epoch: 0,
        };
        let request = ui
            .begin_inline(
                target,
                crate::actions::Mode::Alias,
                Some((11, Some("typed".into()), None)),
            )
            .unwrap();
        let (_, inputs, overflow) = ui.drain();
        assert!(overflow);
        assert_eq!(inputs.len(), 129);
        assert!(
            matches!(&inputs[0].action,Action::BeginInline{previous:Some(p),..} if p.token==request.0 && p.attempt==request.1 && p.aliases.as_deref()==Some("typed"))
        );
    }
    #[test]
    fn keyword_visibility_does_not_wait_for_a_held_presentation_lock() {
        let (events, _) = std::sync::mpsc::sync_channel(1);
        let ui = Connection::new(events, false);
        let held = ui.view.lock().unwrap();
        let reader = ui.clone();
        let (sent, received) = std::sync::mpsc::channel();
        let thread = std::thread::spawn(move || {
            sent.send(reader.palette_visible()).unwrap();
        });
        let observed = received.recv_timeout(std::time::Duration::from_secs(1));
        drop(held);
        thread.join().unwrap();
        assert!(!observed.unwrap());
    }
    #[test]
    fn view_excludes_executable_and_snippet_payloads_and_late_attach_wakes() {
        use std::sync::atomic::AtomicUsize;
        let (tx, _) = std::sync::mpsc::sync_channel(1);
        let ui = Connection::new(tx, false);
        let mut commands = crate::session::fixture(2);
        commands[0].target = "PRIVATE-PAYLOAD".into();
        commands[0].arguments = vec!["PRIVATE-ARGUMENT".into()];
        let session = Session::new(commands);
        let view = View::from_session(&session, 0, 1, None, String::new());
        let text = format!("{view:?}");
        assert!(!text.contains("PRIVATE-"));
        ui.publish(view.clone());
        let calls = Arc::new(AtomicUsize::new(0));
        let count = calls.clone();
        ui.attach_wake(Arc::new(move || {
            count.fetch_add(1, Ordering::Relaxed);
        }));
        assert_eq!(calls.load(Ordering::Relaxed), 1);
        ui.publish(view);
        assert_eq!(calls.load(Ordering::Relaxed), 1);
        ui.query("same text");
        let epoch = ui.latest_epoch();
        let mut view = ui.view();
        view.query = "same text".into();
        view.epoch = epoch;
        ui.publish(view.clone());
        ui.query("same text");
        assert_eq!(ui.latest_epoch(), epoch);
        view.query = "CLI changed".into();
        ui.publish(view);
        ui.query("same text");
        assert_eq!(ui.latest_epoch(), epoch + 1);
    }
    #[test]
    fn retired_result_ingress_cannot_change_intent_epoch_or_submitted_query() {
        let (tx, _rx) = std::sync::mpsc::sync_channel(1);
        let ui = Connection::new(tx, false);
        let surface = crate::script_session::Surface {
            token: 2,
            title: "Current".into(),
            placeholder: "Find".into(),
            query: Some(String::new()),
            query_valid: true,
            rows: vec![],
            selection: None,
            detail: None,
            busy: false,
            detail_visible: false,
            has_choices: true,
            error: String::new(),
        };
        ui.publish(View {
            script_result: Some(surface),
            ..Default::default()
        });
        for token in [1, 3] {
            ui.query_result(token, "stale", false);
            for name in ["close", "details", "choose"] {
                ui.action(Action::Result {
                    token,
                    name: name.into(),
                    id: None,
                    epoch: 0,
                });
            }
        }
        assert_eq!(ui.latest_intent(), 0);
        assert_eq!(ui.latest_epoch(), 0);
        assert!(!ui.result_cancelled(2));
        assert_eq!(ui.submitted.lock().unwrap().as_deref(), Some(""));
        let (query, actions, overflow) = ui.drain();
        assert!(query.is_none() && actions.is_empty() && !overflow);
        ui.query_result(2, "current", false);
        assert_eq!(ui.latest_intent(), 1);
        assert_eq!(ui.latest_epoch(), 1);
        ui.cancel_result(2); // owner retirement before a new view is published
        ui.query_result(2, "retired", false);
        ui.action(Action::Result {
            token: 2,
            name: "close".into(),
            id: None,
            epoch: 1,
        });
        assert_eq!(ui.latest_intent(), 1);
        assert_eq!(ui.latest_epoch(), 1);
        assert_eq!(ui.submitted.lock().unwrap().as_deref(), Some("current"));
    }
}
