//! OS-provider integration with the one runtime/session owner.
use super::*;
#[path="runtime_builtins.rs"]pub(super) mod builtins;
use crate::system_search::{self as os, service::Pending, Application, Results, Scope, Ticket};
use std::{
    collections::{HashMap, VecDeque},
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Mutex,
    },
};

pub(super) struct Shared {
    monitor_effect:Option<(u64,Pending<()>)>,
    pub monitor:crate::monitor::Owner,
    monitor_cli:Arc<Mutex<crate::resource_sampler::Sampler>>,
    sampler_source:Arc<dyn crate::resource_sampler::Source>,
    pub service: os::service::Service,
    pub main_tabs_enabled: bool,
    revision: Arc<AtomicU64>,
    commands: Mutex<(HashMap<String, rustcast_core::Command>, VecDeque<String>)>,
    local_catalog: Mutex<Option<LocalCatalog>>,
    cli: Vec<Cli>,
    #[cfg(any(test, feature = "test-tools"))]
    pub fixture: Option<Arc<os::fixture::Fixture>>,
}
struct LocalCatalog {
    catalog: crate::catalog::Catalog,
    config_revision: u64,
    applications_revision: u64,
    ranking_revision: u64,
    usage_revision: u64,
}
impl Shared {
    pub fn new(fixture: bool) -> Self {
        let sampler_source:Arc<dyn crate::resource_sampler::Source>={
            #[cfg(any(test,feature="test-tools"))]{if fixture{Arc::new(crate::resource_sampler::Fixture::default())}else{Arc::new(crate::resource_sampler::Native)}}
            #[cfg(not(any(test,feature="test-tools")))]{Arc::new(crate::resource_sampler::Native)}
        };
        #[cfg(any(test, feature = "test-tools"))]
        let owned = fixture.then(|| {
            let f = Arc::new(os::fixture::Fixture::default());
            #[cfg(feature="test-tools")]
            if let Some(home)=std::env::var_os("RUYCAST_WORKSPACE_FIXTURE_HOME").map(std::path::PathBuf::from).filter(|home|crate::workspace::fixture_installed(home)) {
                *f.file_paths.lock().unwrap()=["Fixture Document.txt","Fixture Folder"].iter().map(|name|home.join(name).to_string_lossy().into_owned()).collect();
            }
            if std::env::var("RUYCAST_NATIVE_TEST_SYSTEM_SEARCH").as_deref() != Ok("owned") {
                f.tabs.lock().unwrap().clear();
                f.apps.lock().unwrap().clear();
                f.shortcuts.lock().unwrap().clear();
            }
            f
        });
        let platform: Arc<dyn os::Platform> = {
            #[cfg(any(test, feature = "test-tools"))]
            {
                if let Some(f) = &owned {
                    f.clone()
                } else {
                    Arc::new(os::native::Native::new())
                }
            }
            #[cfg(not(any(test, feature = "test-tools")))]
            {
                let _ = fixture;
                Arc::new(os::native::Native::new())
            }
        };
        Self {
            monitor_effect:None,
            monitor:Default::default(),monitor_cli:Default::default(),sampler_source,
            service: os::service::Service::new(platform),
            main_tabs_enabled: false,
            revision: Arc::new(AtomicU64::new(0)),
            commands: Default::default(),
            local_catalog: Default::default(),
            cli: vec![],
            #[cfg(any(test, feature = "test-tools"))]
            fixture: owned,
        }
    }
    pub fn invalidate_if_revision(&self, revision: u64) {
        self.revision.store(revision, Ordering::Release);
    }
    fn pin(&self, revision: u64) -> Arc<AtomicU64> {
        self.revision.store(revision, Ordering::Release);
        self.revision.clone()
    }
    pub(super) fn remember(&self, r: &Results) {
        let mut store = self.commands.lock().unwrap_or_else(|p| p.into_inner());
        for c in &r.commands {
            if !store.0.contains_key(&c.id) {
                if store.1.len() >= 4096 {
                    if let Some(old) = store.1.pop_front() {
                        store.0.remove(&old);
                    }
                }
                store.1.push_back(c.id.clone());
            }
            store.0.insert(c.id.clone(), c.clone());
        }
    }
    pub fn command(&self, id: &str) -> Option<rustcast_core::Command> {
        self.commands
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .0
            .get(id)
            .cloned()
    }
    fn previous(&self) -> Option<Application> {
        #[cfg(any(test, feature = "test-tools"))]
        if let Some(f) = &self.fixture {
            return f.previous();
        }
        None
    }
    fn activate(
        &self,
        state: &SharedState,
        menu: Option<os::Identity>,
    ) -> Result<os::Activate, String> {
        #[cfg(any(test, feature = "test-tools"))]
        if let Some(f) = &self.fixture {
            return Ok(f.activate());
        }
        let services = state
            .services
            .as_ref()
            .ok_or("Native system service unavailable.")?
            .system
            .clone();
        Ok(Arc::new(move |id, t| {
            if let Some(expected) = menu {
                if services.foreground(t)?.map(|a| a.identity) != Some(expected) {
                    return Err("The original app changed. Search its menu again.".into());
                }
            }
            services.activate(id, t)
        }))
    }
    pub fn busy(&self) -> bool {
        !self.cli.is_empty() || self.monitor.busy() || self.monitor_effect.is_some()
    }
}
struct Search {
    stamp: (u64, u64, u64, u64, String),
    pending: Pending<Results>,
    intent: u64,
}
struct ActionJob {
    allow_revision:bool,
    keep_palette:bool,
    process_title:Option<String>,
    pending: ActionPending,
    scope_live: Arc<AtomicBool>,
    epoch: u64,
    intent: u64,
    dismissal: u64,
    revision: u64,
    consent: bool,
    consent_selection_revision:u64,
    record: Option<ActionRecord>,
    hide_key: Option<crate::connected::HideKey>,
    hidden: Arc<AtomicBool>,
    input_token: Option<u64>,
}
enum ActionPending { Action(Pending<()>), Layout(Pending<rustcast_core::Frame>) }
impl ActionPending { fn poll(&self)->Option<Result<(),String>>{match self{Self::Action(p)=>p.poll(),Self::Layout(p)=>p.poll().map(|r|r.map(|_|()))}} fn deadline(&self)->Instant{match self{Self::Action(p)=>p.deadline,Self::Layout(p)=>p.deadline}} }
struct ActionRecord {
    trace: String,
    id: String,
    logged_id: String,
    started: Instant,
    shortcut: bool,
    source: &'static str,
    span: Option<crate::analytics::Span>,
    secrets: Vec<String>,
}
fn record_start(
    state: &mut SharedState,
    c: &rustcast_core::Command,
    source: &'static str,
    values: &Value,
) -> ActionRecord {
    let id = if c.target.starts_with("system.tab:") {
        "system.tab"
    } else if c.target.starts_with("system.window:") {
        "system.window"
    } else if c.target.starts_with("system.kill:") {
        "system.kill"
    } else if c.target.starts_with("system.menu:") {
        "system.menu"
    } else {
        &c.id
    }
    .to_string();
    let logged_id = crate::activity::Activity::log_id(&id, values, &state.configuration.paths.home);
    let trace = uuid();
    state.activity.record(&trace, &logged_id, source, None);
    ActionRecord {
        trace,
        id: id.clone(),
        logged_id,
        started: Instant::now(),
        shortcut: os::apple_shortcuts::recognizes(c) || c.kind=="window" || (c.kind=="builtin"&&!ephemeral(c)),
        source,
        span: crate::analytics::Span::new(state.analytics.client(), &id, source),
        secrets: log_secrets(values),
    }
}
fn record_finish(
    state: &mut SharedState,
    mut record: ActionRecord,
    response: &mut Value,
    current: bool,
) -> Result<(), String> {
    let success = response["ok"] == true;
    if !success { response["diagnosticID"] = json!(record.trace); }
    state.activity.record_response(
        &record.trace,
        &record.logged_id,
        record.source,
        record.started.elapsed().as_secs_f64() * 1000.,
        response,
        (&record.secrets, &state.configuration.paths.home),
    );
    if let Some(span) = record.span.take() {
        span.finish(
            success,
            if success {
                "success"
            } else {
                crate::analytics::failure_code(response["error"].as_str().unwrap_or(""))
            },
        );
    }
    if record.shortcut && success && current {
        state.activity.success(&record.id)?;
        if record.source == "launcher" {
            state
                .ranking
                .record_success(&record.id, crate::activity::now())?;
        }
    }
    Ok(())
}
pub(super) struct Owner {
    pub process_sort:crate::processes::Sort,
    next_refresh:Option<Instant>,
    refreshing:bool,
    pub scope: Scope,
    search: Option<Search>,
    completed: Option<(u64, u64, u64, u64, String)>,
    pub snapshot: Results,
    action: Option<ActionJob>,
    retry: Option<Instant>,
    scope_live: Arc<AtomicBool>,
}
impl Default for Owner {
    fn default() -> Self {
        Self {
            process_sort:Default::default(),next_refresh:None,refreshing:false,
            scope: Scope::MainTabs,
            search: None,
            completed: None,
            snapshot: Default::default(),
            action: None,
            retry: None,
            scope_live: Arc::new(AtomicBool::new(true)),
        }
    }
}
impl Owner {
    pub fn busy(&self) -> bool {
        self.search.is_some() || self.action.is_some()
    }
    pub fn action_running(&self) -> bool {
        self.action.is_some()
    }
    pub fn deadline(&self) -> Option<Instant> {
        self.search
            .as_ref()
            .map(|s| s.pending.deadline)
            .into_iter()
            .chain(self.action.as_ref().map(|a| a.pending.deadline()))
            .chain(self.retry)
            .chain(self.next_refresh)
            .min()
    }
    pub fn refresh(&mut self) {
        self.search = None;
        self.completed = None;
        self.retry = None;
        self.next_refresh=None;
        self.refreshing=false;
    }
    pub fn enter(&mut self, scope: Scope) {
        // Revoke the old ticket before starting another mode, even if its query,
        // epoch and configuration happen to be unchanged. Keep the action until
        // poll_native accounts for its outcome; never let it close the new UI.
        self.scope_live.store(false, Ordering::Release);
        self.scope_live = Arc::new(AtomicBool::new(true));
        self.refresh();
        self.scope = scope;
        self.snapshot = Default::default();
    }
}
fn wake(state: &SharedState) -> Arc<dyn Fn() + Send + Sync> {
    let tx = state.events.clone();
    Arc::new(move || {
        let _ = tx.try_send(HostEvent::UiWake);
    })
}
fn base(state: &SharedState) -> Vec<rustcast_core::Command> {
    crate::applications::merge(
        &crate::management_commands::catalog(&state.configuration.current().commands),
        &state.applications,
    )
}
pub(super) fn local(state: &SharedState, query: &str, limit: usize) -> Vec<rustcast_core::Command> {
    let mut slot = state.system.local_catalog.lock().unwrap_or_else(|p| p.into_inner());
    let prepared = slot.get_or_insert_with(|| {
        let mut catalog = crate::catalog::Catalog::new(base(state));
        catalog.prepare_recency(state.ranking.entries());
        catalog.set_usage(state.activity.counts.clone());
        LocalCatalog { catalog, config_revision: state.configuration.revision,
            applications_revision: state.applications_revision,
            ranking_revision: state.ranking.revision, usage_revision: state.activity.usage_revision }
    });
    if prepared.config_revision != state.configuration.revision
        || prepared.applications_revision != state.applications_revision
    {
        prepared.catalog.replace(base(state));
        prepared.config_revision = state.configuration.revision;
        prepared.applications_revision = state.applications_revision;
    }
    if prepared.ranking_revision != state.ranking.revision {
        prepared.catalog.prepare_recency(state.ranking.entries());
        prepared.ranking_revision = state.ranking.revision;
    }
    if prepared.usage_revision != state.activity.usage_revision {
        prepared.catalog.set_usage(state.activity.counts.clone());
        prepared.usage_revision = state.activity.usage_revision;
    }
    prepared.catalog.search(query, limit)
        .into_iter()
        .map(|i| prepared.catalog.command(i).clone())
        .collect()
}
fn combine(
    state: &SharedState,
    scope: Scope,
    query: &str,
    results: &Results,
    local_limit: usize,
    limit: usize,
) -> Vec<rustcast_core::Command> {
    if scope == Scope::MainTabs {
        let tabs = &results.commands;
        let ids: std::collections::HashSet<_> = tabs.iter().map(|c| &c.id).collect();
        let mut combined: Vec<_> = tabs
            .iter()
            .cloned()
            .chain(
                local(state, query, local_limit)
                    .into_iter()
                    .filter(|c| !ids.contains(&c.id)),
            )
            .collect();
        if limit > 0 {
            combined.truncate(limit);
        }
        combined
    } else if scope == Scope::Shortcuts {
        os::apple_shortcuts::merged(
            &results.commands,
            &state.configuration.current().commands,
            query,
        )
    } else {
        results.commands.clone()
    }
}
pub(super) fn tick(w: &mut WindowRuntime, state: &SharedState, ui: &Arc<Connection>) {
    let revision = state.configuration.revision;
    state.system.pin(revision);
    let active = w.session.visible
        && w.utility.mode.is_none()
        && !w.session.clipboard_mode
        && w.session.launcher_scope.is_none()
        && w.session.library.is_none()
        && w.results.token().is_none()
        && !state.config_busy()
        && !state.config_dirty;
    if !active {
        w.system.refresh();
        return;
    }
    if w.system.scope==Scope::Processes && w.system.next_refresh.is_some_and(|at|Instant::now()>=at){
        w.system.next_refresh=Some(Instant::now()+Duration::from_secs(2));
        if !w.session.actions_open&&w.editing.token().is_none()&&w.system.action.is_none(){w.system.completed=None;w.system.refreshing=true;}
    }
    let stamp = (
        w.epoch,
        revision,
        state.applications_revision,
        w.show_serial,
        w.session.query.clone(),
    );
    if w.system.search.as_ref().is_some_and(|s| s.stamp != stamp) {
        w.system.refresh();
    }
    if let Some(result) = w.system.search.as_ref().and_then(|s| s.pending.poll()) {
        let search = w.system.search.take().unwrap();
        if search.stamp == stamp
            && !ui.stop_requested()
            && ui.latest_epoch() == w.epoch
            && ui.latest_dismissal() == w.dismissal
        {
            w.system.completed = Some(stamp.clone());
            match result {
                Ok(results) => {
                    // A late optional group cannot retarget an open action menu,
                    // input draft or a result the user has already activated.
                    if !w.session.actions_open
                        && w.editing.token().is_none()
                        && w.system.action.is_none()
                    {
                        state.system.remember(&results);
                        let mut rows = combine(state, w.system.scope, &w.session.query, &results, 100, 100);
                        if w.system.scope==Scope::Processes {sort_process_rows(&mut rows,&results,w.system.process_sort);}
                        w.session
                            .set_external_results(rows, w.system.refreshing || search.intent != ui.latest_intent());
                        w.system.snapshot = results;
                    }
                }
                Err(e) => {
                    if w.system.scope != Scope::MainTabs {
                        w.message = e;
                    }
                }
            }
            w.stamp = None;
            if w.system.scope==Scope::Processes {w.system.next_refresh=Some(Instant::now()+Duration::from_secs(2));w.system.refreshing=false;}
            // The consent refresh's selection-preserving handoff is one-shot.
            if w.system.scope==Scope::MainTabs {w.system.refreshing=false;}
        }
    }
    if w.system.search.is_some()
        || w.system.completed.as_ref() == Some(&stamp)
        || w.system.action.is_some()
        || w.system.retry.is_some_and(|d| Instant::now() < d)
    {
        return;
    }
    w.system.retry = None;
    if w.session.actions_open || w.editing.token().is_some() {
        return;
    }
    let scope = w.system.scope;
    let empty = Results::default();
    if !w.system.refreshing {w.session.set_external_results(combine(state, scope, &w.session.query, &empty, 100, 100), false);w.system.snapshot = empty;}
    let config = state.system.pin(revision);
    let weak = Arc::downgrade(ui);
    let epoch = w.epoch;
    let dismissal = w.dismissal;
    let ticket = Ticket {
        deadline: Instant::now() + scope.budget(),
        live: Arc::new(move || {
            config.load(Ordering::Acquire) == revision
                && weak.upgrade().is_some_and(|ui| {
                    !ui.stop_requested()
                        && ui.latest_epoch() == epoch
                        && ui.latest_dismissal() == dismissal
                })
        }),
    };
    let foreground = state.services.as_ref().map(|s| s.system.clone());
    match state.system.service.search_with_foreground(
        scope,
        w.session.query.clone(),
        state.system.previous(),
        foreground,
        ticket,
        wake(state),
    ) {
        Ok(pending) => {
            w.system.search = Some(Search {
                stamp,
                pending,
                intent: ui.latest_intent(),
            })
        }
        Err(_) => w.system.retry = Some(Instant::now() + Duration::from_millis(25)),
    }
    w.stamp = None;
}
pub(super) fn start_native(
    state: &mut SharedState,
    w: &mut WindowRuntime,
    ui: &Arc<Connection>,
    c: &rustcast_core::Command,
) -> Result<(), String> {
    start_native_input(state, w, ui, c, None)
}
pub(super) fn start_native_input(
    state: &mut SharedState,
    w: &mut WindowRuntime,
    ui: &Arc<Connection>,
    c: &rustcast_core::Command,
    input_token: Option<u64>,
) -> Result<(), String> {
    if w.system.action.is_some() {
        return Err("A system action is already running.".into());
    }
    if c.enabled == Some(false) {
        return Err("Command is disabled.".into());
    }
    if c.input_prompt.is_some() && input_token.is_none() {
        if w.native_source == "hotkey" && !w.session.visible {
            w.show(Some(""), Instant::now(), ui)?;
            w.native_source = "hotkey";
        }
        w.editing
            .open(&state.configuration, c, crate::actions::Mode::Input);
        w.editing
            .set_run_inline(w.editing.token().ok_or("Input is unavailable.")?, true);
        w.session.actions_open = false;
        w.message.clear();
        return Ok(());
    }
    start_native_values(state, w, ui, c, &json!({}), input_token)
}
fn action_budget(command: &rustcast_core::Command) -> Result<Duration, String> {
    if os::apple_shortcuts::recognizes(command) {
        let seconds = command.timeout.unwrap_or(30.);
        if !seconds.is_finite() || !(0.1..=300.).contains(&seconds) {
            return Err("Script timeout must be between 0.1 and 300 seconds.".into());
        }
        Ok(Duration::from_secs_f64(seconds))
    } else {
        Ok(Duration::from_secs(
            if command.target.starts_with("system.tab:") {
                8
            } else {
                2
            },
        ))
    }
}
pub(super) fn start_native_values(
    state: &mut SharedState,
    w: &mut WindowRuntime,
    ui: &Arc<Connection>,
    c: &rustcast_core::Command,
    values: &Value,
    input_token: Option<u64>,
) -> Result<(), String> {
    if w.system.action.is_some() {
        return Err("A system action is already running.".into());
    }
    if c.enabled == Some(false) {
        return Err("Command is disabled.".into());
    }
    let known = state
        .system
        .command(&c.id)
        .filter(|k| execution_identity(c, k))
        .or_else(|| {
            state
                .configuration
                .current()
                .commands
                .iter()
                .find(|k| execution_identity(c, k))
                .cloned()
        })
        .ok_or("Selection changed. Try again.")?;
    crate::management_commands::validate_input(&known, values)?;
    let scope_live = w.system.scope_live.clone();
    let ticket_scope_live = scope_live.clone();
    let epoch = ui.latest_epoch();
    let intent = ui.latest_intent();
    let dismissal = ui.latest_dismissal();
    let revision = state.configuration.revision;
    let config = state.system.pin(revision);
    let weak = Arc::downgrade(ui);
    let live = Arc::new(move || {
        ticket_scope_live.load(Ordering::Acquire)
            && config.load(Ordering::Acquire) == revision
            && weak.upgrade().is_some_and(|ui| {
                !ui.stop_requested()
                    && ui.latest_epoch() == epoch
                    && ui.latest_intent() == intent
                    && ui.latest_dismissal() == dismissal
                    && input_token.is_none_or(|token| !ui.editor_cancelled(token))
            })
    });
    let shortcut = os::apple_shortcuts::recognizes(&known);
    let ticket = Ticket {
        deadline: Instant::now() + action_budget(&known)?,
        live,
    };
    let hidden = Arc::new(AtomicBool::new(false));
    let (hide_key, before) = if shortcut || c.target.starts_with("system.tab:") {
        let services = state
            .services
            .as_ref()
            .ok_or("Native window service is unavailable.")?
            .clone();
        let serial = w
            .execution_serial
            .checked_add(1)
            .ok_or("Execution sequence exhausted.")?;
        let key = crate::connected::HideKey {
            trace: uuid(),
            epoch,
            dismissal,
            intent,
            serial,
        };
        let token = ui.arm_open_hide(key.clone())?;
        w.execution_serial = serial;
        let permit = Arc::new(crate::main_services::Permit::default());
        let dispatch = Arc::new(crate::workspace::Dispatch::default());
        let acknowledged = hidden.clone();
        let before: os::Prepare = Arc::new(move |t| {
            t.check()?;
            services.hide(
                token.clone(),
                t.deadline,
                t.live.clone(),
                permit.live(),
                dispatch.clone(),
            )?;
            acknowledged.store(true, Ordering::Release);
            t.check()
        });
        (Some(key), Some(before))
    } else {
        (None, None)
    };
    let pending = if c.kind=="window" {
        state.system.service.layout(c.target.clone(),state.system.previous(),state.services.as_ref().map(|s|s.system.clone()),ticket,wake(state)).map(ActionPending::Layout)
    } else if shortcut {
        state.system.service.run_shortcut_prepared(
            known.arguments[1].clone(),
            ticket,
            before,
            wake(state),
        ).map(ActionPending::Action)
    } else {
        let previous = w.system.snapshot.foreground.as_ref().map(|a| a.identity);
        let activate = state.system.activate(
            state,
            c.target
                .starts_with("system.menu:")
                .then_some(previous)
                .flatten(),
        )?;
        state.system.service.execute_prepared(
            c.target.clone(),
            previous,
            ticket,
            activate,
            before,
            wake(state),
        ).map(ActionPending::Action)
    };
    let pending = match pending {
        Ok(p) => p,
        Err(e) => {
            if let Some(key) = &hide_key {
                ui.finish_open_hide(key);
            }
            return Err(e);
        }
    };
    let record = record_start(state, &known, w.native_source, values);
    w.system.action = Some(ActionJob {
        allow_revision:false,keep_palette:false,
        process_title:c.target.starts_with("system.kill:").then(||c.title.clone()),
        pending,
        scope_live,
        epoch,
        intent,
        dismissal,
        revision,
        consent: false,
        consent_selection_revision:0,
        record: Some(record),
        hide_key,
        hidden,
        input_token,
    });
    w.system.search = None;
    w.session.actions_open = false;
    w.message.clear();
    w.stamp = None;
    Ok(())
}
pub(super) fn consent(
    state: &SharedState,
    w: &mut WindowRuntime,
    ui: &Arc<Connection>,
) -> Result<(), String> {
    if w.session.launcher_scope.is_some() || ![Scope::MainTabs, Scope::Tabs].contains(&w.system.scope) {
        return Err("Safari access is unavailable in this search.".into());
    }
    if w.system.action.is_some() {
        return Err("A system action is already running.".into());
    }
    let scope_live = w.system.scope_live.clone();
    let ticket_scope_live = scope_live.clone();
    let epoch = ui.latest_epoch();
    let intent = ui.latest_intent();
    let main_consent=w.system.scope==Scope::MainTabs;
    let consent_selection_revision=ui.consent_selection_revision();
    let dismissal = ui.latest_dismissal();
    let revision = state.configuration.revision;
    let config = state.system.pin(revision);
    let weak = Arc::downgrade(ui);
    let live = Arc::new(move || {
        ticket_scope_live.load(Ordering::Acquire)
            && config.load(Ordering::Acquire) == revision
            && weak.upgrade().is_some_and(|ui| {
                !ui.stop_requested()
                    && (main_consent || (ui.latest_epoch()==epoch && ui.latest_intent()==intent))
                    && ui.latest_dismissal() == dismissal
            })
    });
    let pending = state.system.service.consent(
        Ticket {
            deadline: Instant::now() + Duration::from_secs(300),
            live,
        },
        wake(state),
    )?;
    w.system.search = None;
    w.system.action = Some(ActionJob {
        allow_revision:false,keep_palette:false,
        process_title:None,
        pending:ActionPending::Action(pending),
        scope_live,
        epoch,
        intent,
        dismissal,
        revision,
        consent: true,
        consent_selection_revision,
        record: None,
        hide_key: None,
        hidden: Arc::new(AtomicBool::new(false)),
        input_token: None,
    });
    w.session.actions_open = false;
    w.stamp = None;
    Ok(())
}
pub(super) fn poll_native(state: &mut SharedState, w: &mut WindowRuntime, ui: &Arc<Connection>) {
    let Some(action) = &w.system.action else {
        return;
    };
    let main_consent=action.consent && w.system.scope==Scope::MainTabs;
    let interaction_clear=!w.session.actions_open && w.editing.token().is_none() && w.results.token().is_none() && w.creation_request.is_none() && w.field_request.is_none();
    let current = action.scope_live.load(Ordering::Acquire)
        && w.utility.mode.is_none()
        && !w.session.clipboard_mode
        && w.session.launcher_scope.is_none()
        && if main_consent {w.session.visible}else{action.epoch==ui.latest_epoch() && action.intent==ui.latest_intent()}
        && action.dismissal == ui.latest_dismissal()
        && (action.allow_revision || action.revision == state.configuration.revision)
        && !ui.stop_requested();
    let done = action.pending.poll();
    if !current || done.is_some() {
        let mut action = w.system.action.take().unwrap();
        if main_consent {w.stamp=None;}
        if let Some(record) = action.record.take() {
            let mut response=match done.as_ref() {Some(Ok(()))=>json!({"ok":true}),Some(Err(error))=>json!({"ok":false,"error":error}),None=>json!({"ok":false,"error":"Configuration changed or request cancelled. Search again."})};
            let _ = record_finish(
                state,
                record,
                &mut response,
                current,
            );
        }
        if let Some(key) = &action.hide_key {
            ui.finish_open_hide(key);
        }
        // An interaction suppresses only the completed refresh. It does not
        // cancel an entered permission request which may finish after closing it.
        if current && (!main_consent || interaction_clear) {
            if let Some(token)=action.input_token {if w.editing.token()==Some(token){w.editing.cancel(token);}}
            match done.unwrap() {
                Ok(()) => {
                    if action.consent {
                        state.system.service.refresh();
                        w.system.refresh();
                        w.system.refreshing=w.system.scope==Scope::MainTabs && (action.epoch!=ui.latest_epoch()||action.consent_selection_revision!=ui.consent_selection_revision());
                    } else if let Some(title)=action.process_title {
                        w.system.refresh();w.system.refreshing=true;w.notify(format!("Killed {title}"),true);
                    } else if action.keep_palette {
                        w.notify("Configuration reloaded".into(),true);
                    } else {
                        if let Some(token) = action.input_token {
                            if w.editing.token() == Some(token) {
                                w.editing.cancel(token);
                            }
                        }
                        w.session.visible = false;
                    }
                    w.message.clear();
                }
                Err(e) => {
                    if action.hidden.load(Ordering::Acquire) || !w.session.visible {
                        w.session.visible = false;
                        w.notify(e.clone(), false);
                    }
                    w.message = e;
                }
            };
            w.stamp = None;
        }
    }
}
struct Cli {
    request: crate::control::Envelope,
    pending: CliPending,
    scope: Scope,
    revision: u64,
    record: Option<ActionRecord>,
}
enum CliPending {
    Builtin(Pending<()>,&'static str),
    Search(Pending<Results>),
    Action(Pending<()>),
    Layout(Pending<rustcast_core::Frame>),
    Monitor(Pending<crate::resource_sampler::Snapshot>),
}
pub(super) fn cli_search(state: &mut SharedState, request: crate::control::Envelope) -> bool {
    let Some(scope) = request.value["mode"]
        .as_str()
        .or(request.value["scope"].as_str())
        .and_then(Scope::parse)
        .or_else(|| {
            request
                .value
                .get("mode")
                .is_none()
                .then_some(Scope::MainTabs)
        })
    else {
        return false;
    };
    if scope == Scope::MainTabs && !state.system.main_tabs_enabled {
        let limit = request.value["limit"].as_i64().unwrap_or(100).max(0) as usize;
        let rows = local(state, request.value["query"].as_str().unwrap_or(""), limit);
        let _ = request.respond(json!({"ok":true,"result":rows}));
        return true;
    }
    let revision = state.configuration.revision;
    let config = state.system.pin(revision);
    if state.system.cli.len() >= 8 {
        if scope==Scope::MainTabs && request.context.is_live() && config.load(Ordering::Acquire)==revision {
            let query=request.value["query"].as_str().unwrap_or("");
            let limit=request.value["limit"].as_i64().unwrap_or(100).max(0) as usize;
            let rows=local(state,query,limit);
            let _=request.respond(json!({"ok":true,"result":rows}));
        } else {
            let _=request.respond(json!({"ok":false,"error":"System search is busy. Refresh to retry."}));
        }
        return true;
    }
    let context = request.context.clone();
    let ticket = Ticket {
        deadline: (Instant::now() + scope.budget()).min(context.deadline()),
        live: Arc::new(move || context.is_live() && config.load(Ordering::Acquire) == revision),
    };
    match state.system.service.search_with_foreground(
        scope,
        request.value["query"].as_str().unwrap_or("").into(),
        state.system.previous(),
        state.services.as_ref().map(|s| s.system.clone()),
        ticket,
        wake(state),
    ) {
        Ok(pending) => state.system.cli.push(Cli {
            request,
            pending: CliPending::Search(pending),
            scope,
            revision,
            record: None,
        }),
        Err(_) if scope==Scope::MainTabs
            && request.context.is_live()
            && state.system.revision.load(Ordering::Acquire)==revision => {
            // Main tabs are optional: a busy provider must not block the
            // current CLI query's local results. Dedicated modes stay strict.
            let query=request.value["query"].as_str().unwrap_or("");
            let limit=request.value["limit"].as_i64().unwrap_or(100).max(0) as usize;
            let rows=local(state,query,limit);
            let _=request.respond(json!({"ok":true,"result":rows}));
        }
        Err(e) => {
            let _ = request.respond(json!({"ok":false,"error":e}));
        }
    }
    true
}
pub(super) fn cli_execute(
    state: &mut SharedState,
    request: crate::control::Envelope,
) -> Result<(), crate::control::Envelope> {
    let id = request.value["id"].as_str().unwrap_or("");
    let key = command_key(id);
    if state
        .configuration
        .current()
        .commands
        .iter()
        .any(|c| c.enabled == Some(false) && name_matches(c, &key))
    {
        let _ = request.respond(json!({"ok":false,"error":"Command is disabled."}));
        return Ok(());
    }
    let command = state
        .configuration
        .current()
        .commands
        .iter()
        .find(|c| name_matches(c, &key))
        .cloned()
        .or_else(|| state.system.command(&key))
        .or_else(||crate::management_commands::catalog(&state.configuration.current().commands).into_iter().find(|c|name_matches(c,&key)));
    let Some(command) = command.filter(|c| c.kind=="window" || ephemeral(c) || os::apple_shortcuts::recognizes(c))
    else {
        return Err(request);
    };
    if command.enabled == Some(false) {
        let _ = request.respond(json!({"ok":false,"error":"Command is disabled."}));
        return Ok(());
    }
    if request.value["dryRun"] == true {
        let _ = request.respond(json!({"ok":true,"result":command,"dryRun":true}));
        return Ok(());
    }
    let values = request
        .value
        .get("values")
        .cloned()
        .unwrap_or_else(|| json!({}));
    if let Err(error) = crate::management_commands::validate_input(&command, &values) {
        let _ = request.respond(json!({"ok":false,"error":error}));
        return Ok(());
    }
    let budget = match action_budget(&command) {
        Ok(budget) => budget,
        Err(error) => {
            let _ = request.respond(json!({"ok":false,"error":error}));
            return Ok(());
        }
    };
    if command.kind=="window" {
        if let Err(e)=crate::builtin_effects::validate(&command,&request.value["values"]){let _=request.respond(json!({"ok":false,"error":e}));return Ok(());}
        let revision=state.configuration.revision;let config=state.system.pin(revision);let context=request.context.clone();
        let ticket=Ticket{deadline:context.deadline().min(Instant::now()+Duration::from_secs(3)),live:Arc::new(move||context.is_live()&&config.load(Ordering::Acquire)==revision)};
        let result=if state.system.cli.len()>=8{Err("System action is busy. Try again.".into())}else{state.system.service.layout(command.target.clone(),state.system.previous(),state.services.as_ref().map(|s|s.system.clone()),ticket,wake(state))};
        match result{Ok(pending)=>{let record=record_start(state,&command,"cli",&values);state.system.cli.push(Cli{request,pending:CliPending::Layout(pending),scope:Scope::MainTabs,revision,record:Some(record)});},Err(e)=>{let _=request.respond(json!({"ok":false,"error":e}));}}
        return Ok(());
    }
    let revision = state.configuration.revision;
    let config = state.system.pin(revision);
    let context = request.context.clone();
    let shortcut = os::apple_shortcuts::recognizes(&command);
    if shortcut {
        if let Err(error) = context.bind_shortcut() {
            let _ = request.respond(json!({"ok":false,"error":error}));
            return Ok(());
        }
    }
    let ticket = Ticket {
        deadline: context.deadline().min(Instant::now() + budget),
        live: Arc::new(move || context.is_live() && config.load(Ordering::Acquire) == revision),
    };
    let result = if state.system.cli.len() >= 8 {
        Err("System search is busy. Refresh to retry.".into())
    } else if shortcut {
        state
            .system
            .service
            .run_shortcut(command.arguments[1].clone(), ticket, wake(state))
    } else {
        // The request must explicitly carry the original captured foreground
        // identity for menu effects. No caller PID or title grants authority.
        let previous = request
            .value
            .get("foreground")
            .cloned()
            .and_then(|v| serde_json::from_value::<os::Identity>(v).ok());
        match state.system.activate(
            state,
            command
                .target
                .starts_with("system.menu:")
                .then_some(previous)
                .flatten(),
        ) {
            Ok(activate) => state.system.service.execute(
                command.target.clone(),
                previous,
                ticket,
                activate,
                wake(state),
            ),
            Err(e) => Err(e),
        }
    };
    match result {
        Ok(pending) => {
            let record = record_start(state, &command, "cli", &values);
            state.system.cli.push(Cli {
                request,
                pending: CliPending::Action(pending),
                scope: Scope::MainTabs,
                revision,
                record: Some(record),
            });
        }
        Err(e) => {
            let _ = request.respond(json!({"ok":false,"error":e}));
        }
    }
    Ok(())
}
pub(super) fn poll_cli(state: &mut SharedState) {
    if let Some(result)=state.system.monitor_effect.as_ref().and_then(|(_,pending)|pending.poll()){let(token,_)=state.system.monitor_effect.take().unwrap();if let Err(error)=result{state.system.monitor.fail(token,error);}}
    let event=wake(state);state.system.monitor.tick(&state.system.service,state.system.sampler_source.clone(),event);
    state.system.pin(state.configuration.revision);
    state.system.service.reap();
    let mut remaining = vec![];
    for mut job in std::mem::take(&mut state.system.cli) {
        if !job.request.context.is_live() || (job.revision != state.configuration.revision && !matches!(job.pending,CliPending::Builtin(_,"reloaded"))) {
            let shortcut = job.record.as_ref().is_some_and(|record| record.shortcut);
            let error = if shortcut {
                "Shortcut run was interrupted. Its result is unknown. Check Shortcuts before retrying."
            } else {
                "Configuration changed or request cancelled. Search again."
            };
            let mut response=json!({"ok":false,"error":error});
            if let Some(record)=job.record.take(){let _=record_finish(state,record,&mut response,false);}
            let _ = job.request.respond(response);
            continue;
        }
        let response = match &job.pending {
            CliPending::Builtin(p,state)=>p.poll().map(|r|r.map(|_|json!({"ok":true,"result":{"state":state}}))),
            CliPending::Search(p) => p.poll().map(|r| {
                let r = if job.scope == Scope::MainTabs {
                    r.or_else(|_| Ok(Results::default()))
                } else {
                    r
                };
                r.map(|mut r| {
                    state.system.remember(&r);
                    let q = job.request.value["query"].as_str().unwrap_or("");
                    let limit = job.request.value["limit"].as_i64().unwrap_or(100).max(0) as usize;
                    r.commands = combine(state, job.scope, q, &r, 0, limit);
                    let mut value = json!({"ok":true,"result":r.json()});
                    if job.request.value["processDetails"]==true {value["result"]=Value::Array(r.commands.iter().filter_map(|c|match r.metadata.get(&c.id){Some(os::Metadata::Process{process})=>Some(json!({"command":c,"metrics":process})),_=>None}).collect());}
                    if let Some(a) = r.foreground {
                        value["foreground"] = json!(a.identity);
                    }
                    value
                })
            }),
            CliPending::Action(p) => p
                .poll()
                .map(|r| r.map(|_| json!({"ok":true,"result":{"state":"completed"}}))),
            CliPending::Layout(p)=>p.poll().map(|r|r.map(|frame|json!({"ok":true,"result":frame}))),
            CliPending::Monitor(p)=>p.poll().map(|r|r.map(|sample|json!({"ok":true,"result":sample}))),
        };
        if let Some(r) = response {
            let mut response=r.unwrap_or_else(|e|json!({"ok":false,"error":e}));
            if let Some(record) = job.record.take() {
                let _ = record_finish(state, record, &mut response, true);
            }
            let _ = job
                .request
                .respond(response);
        } else {
            remaining.push(job);
        }
    }
    state.system.cli = remaining;
}
pub(super) fn cli_monitor(state:&mut SharedState,request:crate::control::Envelope){
    if state.system.cli.len()>=8{let _=request.respond(json!({"ok":false,"error":"System Monitor is busy. Try again."}));return;}
    let revision=state.configuration.revision;let config=state.system.pin(revision);let context=request.context.clone();
    let ticket=Ticket{deadline:context.deadline().min(Instant::now()+Duration::from_secs(3)),live:Arc::new(move||context.is_live()&&config.load(Ordering::Acquire)==revision)};
    let sampler=state.system.monitor_cli.clone();let source=state.system.sampler_source.clone();
    match state.system.service.resource(ticket,wake(state),move|ticket|{ticket.check()?;let sample=sampler.lock().unwrap_or_else(|p|p.into_inner()).sample(&*source);ticket.check()?;Ok(sample)}){
        Ok(pending)=>state.system.cli.push(Cli{request,pending:CliPending::Monitor(pending),scope:Scope::MainTabs,revision,record:None}),Err(error)=>{let _=request.respond(json!({"ok":false,"error":error}));}
    }
}
pub(super) fn open_monitor(state:&mut SharedState,window:&mut WindowRuntime,command:&rustcast_core::Command)->Result<(),String>{
    let record=record_start(state,command,window.native_source,&json!({}));let result=state.system.monitor.open();let mut response=match &result{Ok(_)=>json!({"ok":true}),Err(error)=>json!({"ok":false,"error":error})};let _=record_finish(state,record,&mut response,true);result?;window.session.visible=false;window.stamp=None;Ok(())
}
pub(super) fn monitor_operation(state:&mut SharedState,token:u64,operation:crate::monitor::Operation){
    if !state.system.monitor.operation(token,operation){return;}
    if operation==crate::monitor::Operation::Close{state.system.monitor_effect=None;}
    if operation==crate::monitor::Operation::ActivityMonitor{
        if state.system.monitor_effect.is_some(){return;}
        let services=state.services.as_ref().map(|s|s.system.clone());let live=state.system.monitor.live();let ticket=Ticket{deadline:Instant::now()+Duration::from_secs(3),live};
        match state.system.service.effect(ticket,wake(state),move|ticket|services.ok_or("Native system service unavailable.")?.effect(crate::system_actions::Effect::ActivityMonitor,&ticket)){Ok(pending)=>state.system.monitor_effect=Some((token,pending)),Err(error)=>state.system.monitor.fail(token,error)}
    }
}
pub(super) fn cli_open_builtin(state:&mut SharedState,window:&mut WindowRuntime,ui:Option<&Arc<Connection>>,request:crate::control::Envelope)->Result<(),crate::control::Envelope>{
    builtins::cli(state,window,ui,request)
}
pub(super) fn ephemeral(c: &rustcast_core::Command) -> bool {
    ["system.tab:", "system.menu:", "system.window:","system.kill:"]
        .iter()
        .any(|p| c.target.starts_with(p))
}

pub(super) fn builtin_scope(target: &str) -> Option<Scope> {
    Some(match target {
        "search-menu-items" => Scope::Menus,
        "switch-windows" => Scope::Windows,
        "search-safari-tabs" => Scope::Tabs,
        "apple-shortcuts" => Scope::Shortcuts,
        "search-files"=>Scope::Files,
        "kill-process"=>Scope::Processes,
        _ => return None,
    })
}

pub(super) fn subtitles(results: &Results) -> HashMap<String, String> {
    let mut map = HashMap::new();
    for (id, m) in &results.metadata {
        map.insert(
            id.clone(),
            match m {
                os::Metadata::Menu { menu } => {
                    if menu.path.is_empty() {
                        menu.application.clone()
                    } else {
                        format!("{} › {}", menu.application, menu.path)
                    }
                }
                os::Metadata::Window { window } => format!(
                    "{}{}",
                    window.application,
                    if window.minimized == Some(true) {
                        " · Minimized"
                    } else {
                        ""
                    }
                ),
                os::Metadata::Tab { tab } => tab.url.clone(),
                os::Metadata::Process{process}=>format!("{} · {:.1}% CPU · {}",process.identity.pid,process.cpu_percent,byte_label(process.memory_bytes)),
            },
        );
    }
    for c in &results.commands {
        if os::apple_shortcuts::recognizes(c) {
            let duplicate = results
                .commands
                .iter()
                .filter(|r| r.title == c.title)
                .count()
                > 1;
            map.insert(
                c.id.clone(),
                if duplicate {
                    format!("Apple Shortcuts · {}", &c.id[c.id.len() - 8..])
                } else {
                    "Apple Shortcuts".into()
                },
            );
        }
    }
    map
}
fn byte_label(bytes:u64)->String{crate::resource_sampler::memory_label(bytes)}
fn sort_process_rows(rows:&mut[rustcast_core::Command],results:&Results,sort:crate::processes::Sort){rows.sort_by(|a,b|match(results.metadata.get(&a.id),results.metadata.get(&b.id)){(Some(os::Metadata::Process{process:a}),Some(os::Metadata::Process{process:b}))=>crate::processes::compare(a,b,sort),_=>a.id.cmp(&b.id)});}
pub(super) fn process_sort(w:&mut WindowRuntime,sort:crate::processes::Sort){w.system.process_sort=sort;let mut rows=w.session.rows().cloned().collect::<Vec<_>>();sort_process_rows(&mut rows,&w.system.snapshot,sort);w.session.set_external_results(rows,true);w.stamp=None;}

pub(super) fn extend_items(items: &mut Vec<crate::actions::Item>, window: &WindowRuntime) {
    if window.session.library.is_none()
        && window.session.launcher_scope.is_none()
        && window.utility.mode.is_none()
        && !window.session.clipboard_mode
        && window.editing.token().is_none()
        && window.results.token().is_none()
        && (window.system.scope != Scope::MainTabs
            || window.session.selected().is_none_or(|command|
                matches!(window.system.snapshot.metadata.get(&command.id), Some(os::Metadata::Tab { .. }))))
    {
        items.push(crate::actions::Item {
            id: "refresh-system",
            title: "Refresh".into(),
            shortcut: None,
        });
        if [Scope::MainTabs, Scope::Tabs].contains(&window.system.scope) {
            items.push(crate::actions::Item {
                id: "safari-access",
                title: "Allow Safari Access…".into(),
                shortcut: None,
            });
        }
    }
}

#[cfg(feature = "test-tools")]
pub(super) fn fixture(state: &SharedState, request: &Value) -> Value {
    let Some(f) = &state.system.fixture else {
        return json!({"ok":false,"error":"Owned OS fixture unavailable."});
    };
    if std::env::var("RUYCAST_NATIVE_TEST_SYSTEM_SEARCH").as_deref() != Ok("owned") {
        return json!({"ok":false,"error":"Owned OS fixture not enabled."});
    }
    match request["action"].as_str().unwrap_or("read") {
        "bridge-audit"=>return json!({"ok":true,"result":state.services.as_ref().map(|s|s.system.fixture_audit())}),
        "bridge-startup-probe"=>{
            let Some(services)=state.services.as_ref() else{return json!({"ok":false,"error":"Owned main service unavailable."});};
            let ticket=Ticket{deadline:Instant::now()+Duration::from_secs(2),live:Arc::new(||true)};
            let result=(||{let foreground=services.system.foreground(&ticket)?;let screens=services.system.screens(&ticket)?;services.system.effect(crate::system_actions::Effect::ActivityMonitor,&ticket)?;Ok::<_,String>(json!({"foregroundAbsent":foreground.is_none(),"screens":screens,"audit":services.system.fixture_audit()}))})();
            return match result{Ok(result)=>json!({"ok":true,"result":result}),Err(error)=>json!({"ok":false,"error":error})};
        },
        "read" => {}
        "process-cpu" => {
            let Ok(identity) = serde_json::from_value::<os::Identity>(request["identity"].clone()) else {
                return json!({"ok":false,"error":"Invalid owned process identity."});
            };
            let Some(cpu) = request["cpuPercent"].as_f64().filter(|v|v.is_finite() && *v>=0.) else {
                return json!({"ok":false,"error":"Invalid owned process CPU sample."});
            };
            let Ok(mut rows) = f.process_fixture.rows.lock() else {
                return json!({"ok":false,"error":"Owned process fixture unavailable."});
            };
            let Some(row) = rows.iter_mut().find(|row|row.identity==identity) else {
                return json!({"ok":false,"error":"Owned process identity not found."});
            };
            row.cpu_percent=cpu;
        }
        "clear-effects" => f.effects.lock().unwrap().clear(),
        "trust" => {
            let Some(v) = request["value"].as_bool() else {
                return json!({"ok":false});
            };
            f.trusted.store(v, Ordering::Release);
        }
        "delay" => f.delay_ms.store(
            request["milliseconds"].as_u64().unwrap_or(0).min(5000) as usize,
            Ordering::Release,
        ),
        "rename-window" => f.edit(os::Element(2), |n| {
            n.strings
                .insert("AXTitle".into(), "Changed Owned Window".into());
        }),
        "rename-menu" => f.edit(os::Element(7), |n| {
            n.strings
                .insert("AXTitle".into(), "Changed Owned Menu".into());
        }),
        "menu-variant" => {
            let variant=request["variant"].as_str().unwrap_or("");
            if !["enabled","disabled","long","empty","denied","loading","refreshed","first-only","second-only"].contains(&variant) {
                return json!({"ok":false,"error":"Unknown owned menu fixture variant."});
            }
            f.delay_ms.store(if variant=="loading"{500}else{0},Ordering::Release);
            f.trusted.store(variant!="denied",Ordering::Release);
            let identity=os::Identity{pid:730001,uid:unsafe{libc::getuid()},seconds:11,micros:22};
            *f.apps.lock().unwrap()=vec![os::Application{identity,name:match variant{"long"=>"A Long Application Name","refreshed"=>"Refreshed Fixture",_=>"TextEdit Fixture"}.into(),bundle_identifier:Some("invalid.ruycast.owned".into())}];
            *f.application_roots.lock().unwrap()=[(identity.pid,os::Element(1))].into();
            let node=|title:&str,role:&str,enabled:Option<bool>,children:Vec<os::Element>|{
                let mut value=os::fixture::Node{pid:identity.pid,..Default::default()};
                value.strings.insert("AXTitle".into(),title.into());value.strings.insert("AXRole".into(),role.into());
                if let Some(enabled)=enabled{value.bools.insert("AXEnabled".into(),enabled);}
                value.lists.insert("AXChildren".into(),children);value
            };
            let mut nodes=f.nodes.lock().unwrap();
            nodes.entry(os::Element(1)).or_default().pid=identity.pid;
            nodes.get_mut(&os::Element(1)).unwrap().elements.insert("AXMenuBar".into(),os::Element(4));
            nodes.insert(os::Element(4),node("","AXMenuBar",Some(true),vec![os::Element(5)]));
            nodes.insert(os::Element(5),node("File","AXMenuBarItem",Some(true),vec![os::Element(6)]));
            let mut children=if ["empty","loading"].contains(&variant){vec![]}else{vec![os::Element(7),os::Element(9),os::Element(8),os::Element(10),os::Element(11)]};
            if variant=="long" {
                children[0]=os::Element(40);
                nodes.insert(os::Element(40),node("Export","AXMenuItem",Some(true),vec![os::Element(41)]));
                nodes.insert(os::Element(41),node("","AXMenu",Some(true),vec![os::Element(42)]));
                nodes.insert(os::Element(42),node("Presentation and Sharing","AXMenuItem",Some(true),vec![os::Element(43)]));
                nodes.insert(os::Element(43),node("","AXMenu",Some(true),vec![os::Element(7)]));
            }
            nodes.insert(os::Element(6),node("","AXMenu",Some(true),children));
            let mut first=node(if variant=="long"{"Export Presentation with All Linked Resources…"}else{"Open…"},"AXMenuItem",Some(variant!="second-only"),vec![]);
            first.strings.insert("AXMenuItemCmdChar".into(),if variant=="long"{"+"}else{"o"}.into());
            first.ints.insert("AXMenuItemCmdModifiers".into(),if variant=="long"{7}else{0});
            let mut second=node("Open…","AXMenuItem",Some(variant!="first-only"),vec![]);
            second.strings.insert("AXMenuItemCmdChar".into(),"o".into());second.ints.insert("AXMenuItemCmdModifiers".into(),0);
            nodes.insert(os::Element(7),first);nodes.insert(os::Element(9),second);
            nodes.insert(os::Element(8),node("Save","AXMenuItem",Some(false),vec![]));
            nodes.insert(os::Element(10),node("Unknown State","AXMenuItem",None,vec![]));
            let mut unreadable=node("Unreadable Submenu","AXMenuItem",Some(true),vec![os::Element(12)]);unreadable.unreadable_lists=true;
            nodes.insert(os::Element(11),unreadable);nodes.insert(os::Element(12),node("Hidden Child","AXMenuItem",Some(true),vec![]));
            drop(nodes);
            state.system.service.refresh();
        }
        "window-variant" => {
            let variant=request["variant"].as_str().unwrap_or("");
            if !["normal","minimized","long","empty","denied","loading"].contains(&variant) {
                return json!({"ok":false,"error":"Unknown owned window fixture variant."});
            }
            f.delay_ms.store(if variant=="loading"{500}else{0},Ordering::Release);
            f.trusted.store(variant!="denied",Ordering::Release);
            let first=os::Identity{pid:730001,uid:unsafe{libc::getuid()},seconds:11,micros:22};
            let second=os::Identity{pid:730002,..first};
            *f.apps.lock().unwrap()=vec![
                os::Application{identity:first,name:if variant=="long"{"A Long Application Name"}else{"Editor Fixture"}.into(),bundle_identifier:Some("invalid.ruycast.owned".into())},
                os::Application{identity:second,name:"Browser Fixture".into(),bundle_identifier:Some("invalid.ruycast.owned.secondary".into())},
            ];
            *f.application_roots.lock().unwrap()=[(first.pid,os::Element(1)),(second.pid,os::Element(30))].into();
            let mut nodes=f.nodes.lock().unwrap();
            let mut app=os::fixture::Node{pid:second.pid,..Default::default()};
            app.strings.insert("AXRole".into(),"AXApplication".into());
            app.lists.insert("AXWindows".into(),if variant=="empty"{vec![]}else{vec![os::Element(33)]});
            nodes.insert(os::Element(30),app);
            nodes.get_mut(&os::Element(1)).unwrap().lists.insert("AXWindows".into(),if variant=="empty"{vec![]}else{vec![os::Element(2),os::Element(31),os::Element(3),os::Element(32)]});
            for (id,pid,title,minimized) in [
                (2,first.pid,if variant=="long"{"Project Notes with a Very Long Document Name and Unsaved Changes"}else{"Project Notes"},Some(false)),
                (31,first.pid,"Project Notes",Some(false)),(3,first.pid,"Draft",Some(true)),
                (32,first.pid,"Inspector",None),(33,second.pid,"",None),
            ] {
                let mut node=os::fixture::Node{pid,..Default::default()};
                node.strings.insert("AXRole".into(),"AXWindow".into());node.strings.insert("AXTitle".into(),title.into());
                node.bools.insert("AXEnabled".into(),true);
                if let Some(value)=minimized {node.bools.insert("AXMinimized".into(),value);}
                nodes.insert(os::Element(id),node);
            }
            drop(nodes);
            state.system.service.refresh();
        }
        "shortcut-variant" => {
            let variant=request["variant"].as_str().unwrap_or("");
            if !["normal","duplicate","long","empty","error","loading"].contains(&variant) {
                return json!({"ok":false,"error":"Unknown owned shortcut fixture variant."});
            }
            f.delay_ms.store(if variant=="loading"{500}else{0},Ordering::Release);
            f.shortcuts_failure.store(variant=="error",Ordering::Release);
            let source=match variant {
                "duplicate"=>"Focus (11111111-2222-3333-4444-555555555555)\nFocus (99999999-8888-7777-6666-555555555555)\n",
                "long"=>"Focus (aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee)\nA Very Long Shortcut Name for a Daily Focus Routine and Workspace Setup (11111111-2222-3333-4444-555555555555)\nDaily Notes (99999999-8888-7777-6666-555555555555)\n",
                "empty"|"error"|"loading"=>"",
                _=>"Focus (aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee)\nFocus (11111111-2222-3333-4444-555555555555)\nDaily Notes (99999999-8888-7777-6666-555555555555)\n",
            };
            *f.shortcuts.lock().unwrap()=match os::apple_shortcuts::parse(source.as_bytes()){Ok(rows)=>rows,Err(error)=>return json!({"ok":false,"error":error})};
            state.system.service.refresh();
        }
        "browser-variant" => {
            let variant=request["variant"].as_str().unwrap_or("");
            if !["normal","duplicate","long","empty","denied","loading"].contains(&variant) {
                return json!({"ok":false,"error":"Unknown owned browser fixture variant."});
            }
            f.delay_ms.store(if variant=="loading"{500}else{0},Ordering::Release);
            f.safari_denied.store(variant=="denied",Ordering::Release);
            let mut tabs=vec![
                os::Tab{window:101,index:1,title:"Project Notes".into(),url:"https://example.test/notes".into()},
                os::Tab{window:101,index:2,title:"Project Notes".into(),url:"https://example.test/other".into()},
            ];
            if variant=="long" {
                tabs[0].title="A Very Long Safari Tab Title for Project Notes and a Daily Workspace Review".into();
                tabs[0].url="https://example.test/projects/a-long-address-for-workspace-notes-and-daily-review".into();
            }
            if ["empty","denied","loading"].contains(&variant){tabs.clear();}
            *f.tabs.lock().unwrap()=tabs;
            state.system.service.refresh();
        }
        "rename-tab" => {
            if let Some(tab) = f.tabs.lock().unwrap().first_mut() {
                tab.title = "Changed Owned Tab".into();
            }
        }
        "restart" => f.apps.lock().unwrap()[0].identity.seconds += 1,
        "empty-tabs" => f.tabs.lock().unwrap().clear(),
        _ => return json!({"ok":false,"error":"Unknown owned fixture action."}),
    }
    json!({"ok":true,"result":{"reads":f.reads.load(Ordering::Acquire),"effects":f.effects.lock().unwrap().clone()}})
}

#[cfg(test)]
mod shortcut_budget_tests {
    use super::*;
    #[test]
    fn system_action_error_log_retains_worker_message_and_response_trace() {
        use std::io::{BufRead, BufReader, Write}; use std::os::unix::net::UnixStream;
        let mut case = BrowserCase::new();
        let (events, requests) = mpsc::sync_channel(8);
        let server = crate::control::ControlServer::start(&case.state.configuration.paths.root, events, Default::default()).unwrap();
        let socket = case.state.configuration.paths.root.join("control.sock");
        let client = std::thread::spawn(move || {
            let mut stream = UnixStream::connect(socket).unwrap(); stream.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
            writeln!(stream, "{}", json!({"op":"execute","id":"owned.system.error"})).unwrap();
            let mut line = String::new(); BufReader::new(stream).read_line(&mut line).unwrap(); serde_json::from_str::<Value>(&line).unwrap()
        });
        let request = match requests.recv_timeout(Duration::from_secs(1)).unwrap() { HostEvent::Request(request) => request, _ => panic!("owned request") };
        let command: rustcast_core::Command = serde_json::from_value(json!({"id":"owned.system.error","title":"Owned","kind":"builtin","target":"analytics"})).unwrap();
        let record = record_start(&mut case.state, &command, "cli", &json!({}));
        let pending = case.state.system.service.effect(Ticket { deadline: Instant::now()+Duration::from_secs(2), live: Arc::new(||true) }, Arc::new(||{}), |_|Err("Owned action unavailable token=private-token".into())).unwrap();
        let revision = case.state.configuration.revision;
        case.state.system.cli.push(Cli {request,pending:CliPending::Action(pending),scope:Scope::MainTabs,revision,record:Some(record)});
        let deadline = Instant::now()+Duration::from_secs(2);
        while !case.state.system.cli.is_empty() { poll_cli(&mut case.state); assert!(Instant::now()<deadline); std::thread::sleep(Duration::from_millis(1)); }
        let response = client.join().unwrap(); drop(server);
        assert_eq!(response["error"], "Owned action unavailable token=private-token");
        let logs = case.state.activity.entries(&json!({"commandID":"owned.system.error","errorsOnly":true}));
        assert_eq!(logs[0]["message"], "Owned action unavailable token=[redacted]");
        assert_eq!(logs[0]["code"], "unavailable");
        assert!(response["diagnosticID"].is_string());
        assert_eq!(logs[0]["traceID"], response["diagnosticID"]);
        assert!(case.fixture.effects.lock().unwrap().is_empty());
    }
    type BrowserHold = (mpsc::SyncSender<()>, mpsc::Receiver<()>);
    struct HeldConsent {
        browser_hold: Mutex<Option<BrowserHold>>,
        denied:bool,
        fixture:Arc<os::fixture::Fixture>,
        entered:mpsc::SyncSender<()>,
        release:Mutex<mpsc::Receiver<()>>,
    }
    impl os::Platform for HeldConsent {
        fn identity(&self,pid:i32)->Option<os::Identity>{self.fixture.identity(pid)}
        fn trusted(&self)->bool{self.fixture.trusted()}
        fn applications(&self,t:&Ticket)->Result<Vec<Application>,String>{self.fixture.applications(t)}
        fn application(&self,pid:i32,t:&Ticket)->Result<os::Element,String>{self.fixture.application(pid,t)}
        fn pid(&self,n:os::Element)->Option<i32>{self.fixture.pid(n)}
        fn string(&self,n:os::Element,k:&str,t:&Ticket)->Result<Option<String>,String>{self.fixture.string(n,k,t)}
        fn boolean(&self,n:os::Element,k:&str,t:&Ticket)->Result<Option<bool>,String>{self.fixture.boolean(n,k,t)}
        fn integer(&self,n:os::Element,k:&str,t:&Ticket)->Result<Option<i64>,String>{self.fixture.integer(n,k,t)}
        fn elements(&self,n:os::Element,k:&str,l:usize,e:bool,t:&Ticket)->Result<Vec<os::Element>,String>{self.fixture.elements(n,k,l,e,t)}
        fn element(&self,n:os::Element,k:&str,t:&Ticket)->Result<Option<os::Element>,String>{self.fixture.element(n,k,t)}
        fn same(&self,a:os::Element,b:os::Element)->bool{self.fixture.same(a,b)}
        fn restore(&self,n:os::Element,t:&Ticket)->Result<(),String>{self.fixture.restore(n,t)}
        fn action(&self,n:os::Element,a:&str,t:&Ticket)->Result<(),String>{self.fixture.action(n,a,t)}
        fn safari(&self,t:&Ticket)->Result<os::Identity,String>{self.fixture.safari(t)}
        fn safari_windows(&self,b:os::Identity,t:&Ticket)->Result<Vec<i32>,String>{
            let held=self.browser_hold.lock().unwrap().take();
            if let Some((entered,release))=held {
                entered.send(()).map_err(|_|"owned browser entry disconnected")?;
                release.recv_timeout(Duration::from_secs(5)).map_err(|_|"owned browser release timed out")?;
            }
            self.fixture.safari_windows(b,t)
        }
        fn safari_tabs(&self,b:os::Identity,w:i32,t:&Ticket)->Result<Vec<os::Tab>,String>{self.fixture.safari_tabs(b,w,t)}
        fn safari_select(&self,b:os::Identity,v:&os::Tab,t:&Ticket)->Result<(),String>{self.fixture.safari_select(b,v,t)}
        fn safari_selected(&self,b:os::Identity,v:&os::Tab,t:&Ticket)->Result<bool,String>{self.fixture.safari_selected(b,v,t)}
        fn safari_front_window(&self,b:os::Identity,w:i32,t:&Ticket)->Result<(),String>{self.fixture.safari_front_window(b,w,t)}
        fn shortcuts(&self,t:&Ticket)->Result<Vec<rustcast_core::Command>,String>{self.fixture.shortcuts(t)}
        fn safari_consent(&self,t:&Ticket)->Result<(),String>{
            self.fixture.safari_consent(t)?;
            if !self.denied {self.fixture.safari_denied.store(false,Ordering::Release);}
            self.entered.send(()).map_err(|_|"owned consent entry disconnected")?;
            self.release.lock().unwrap().recv_timeout(Duration::from_secs(2)).map_err(|_|"owned consent release timed out")?;
            // An already-entered OS request may complete after cancellation.
            // Only its publication is revocable; this never calls real TCC.
            if self.denied {os::native::complete_safari_consent(t,||-1743)}else{Ok(())}
        }
    }
    fn held_consent_completion(change:&str)->(bool,Option<String>,Option<String>){
        let (_temp,mut state,_,_,_)=super::super::inline_tests::setup();
        let fixture=Arc::new(os::fixture::Fixture::default());
        fixture.safari_denied.store(true,Ordering::Release);
        fixture.tabs.lock().unwrap().push(os::Tab{window:51,index:2,title:"Owned replacement Tab".into(),url:"https://example.invalid/replacement".into()});
        let (entered,wait)=mpsc::sync_channel(1);let (release,hold)=mpsc::sync_channel(1);
        state.system.service=os::service::Service::new(Arc::new(HeldConsent{browser_hold:Default::default(),denied:change=="denied",fixture:fixture.clone(),entered,release:Mutex::new(hold)}));
        state.system.fixture=Some(fixture.clone());
        state.system.main_tabs_enabled=true;
        let (events,completed)=mpsc::sync_channel(8);state.events=events;
        let (ui_tx,_ui_rx)=mpsc::sync_channel(8);let ui=Connection::new(ui_tx,true);
        let mut window=WindowRuntime::new(crate::session::Session::new(vec![os::command("fixture.first".into(),"Owned first".into(),None),os::command("fixture.second".into(),"Owned second".into(),None)]),Default::default(),&state);
        state.applications=window.session.rows().cloned().collect();
        window.session.visible=true;
        ui.query("Owned");assert!(!process_inputs(&mut state,&mut window,&ui));
        let initial_deadline=Instant::now()+Duration::from_secs(2);
        loop {
            tick(&mut window,&state,&ui);
            if window.system.search.is_none()&&window.system.completed.is_some(){break;}
            assert!(Instant::now()<initial_deadline,"initial owned denied search did not complete");
            std::thread::sleep(Duration::from_millis(1));
        }
        completed.recv_timeout(Duration::from_secs(1)).unwrap();
        assert!(window.system.snapshot.commands.is_empty());
        assert_eq!(window.session.rows().map(|c|c.id.as_str()).collect::<Vec<_>>(),["fixture.first","fixture.second"]);
        consent(&state,&mut window,&ui).unwrap();
        wait.recv_timeout(Duration::from_secs(1)).unwrap();
        assert_eq!(*fixture.effects.lock().unwrap(),["consent"]);
        match change {
            "arrow"=>{ui.action(Action::Key("moveDown:".into()));assert!(!process_inputs(&mut state,&mut window,&ui));}
            "down-up"=>{ui.action(Action::Key("moveDown:".into()));ui.action(Action::Key("moveUp:".into()));assert!(!process_inputs(&mut state,&mut window,&ui));}
            "select"=>{ui.action(Action::Select{id:Some("fixture.second".into()),catalog_revision:window.session.results_revision(),epoch:window.epoch});assert!(!process_inputs(&mut state,&mut window,&ui));}
            "default"|"denied"=>(),
            "actions"=>{ui.action(Action::ToggleActions);assert!(!process_inputs(&mut state,&mut window,&ui));}
            "actions-closed"=>{ui.action(Action::ToggleActions);ui.action(Action::ToggleActions);assert!(!process_inputs(&mut state,&mut window,&ui));}
            "actions-cycle"=>{
                ui.action(Action::ToggleActions);assert!(!process_inputs(&mut state,&mut window,&ui));
                let selection_revision=ui.consent_selection_revision();
                ui.action(Action::Key("moveDown:".into()));assert!(!process_inputs(&mut state,&mut window,&ui));
                assert_eq!(ui.consent_selection_revision(),selection_revision,"menu navigation changed palette selection revision");
                poll_native(&mut state,&mut window,&ui);
                assert!(window.system.action.is_some(),"an interaction must not cancel an entered consent before it completes");
                ui.action(Action::ToggleActions);assert!(!process_inputs(&mut state,&mut window,&ui));
            }
            "editor"|"editor-cycle"=>{
                let command=state.configuration.current().commands.iter().find(|c|c.id=="app.finder").unwrap().clone();
                window.editing.transition_inline(&mut state.configuration,&command,crate::actions::Mode::Alias,None,||true).unwrap();
                let revision=ui.consent_selection_revision();
                ui.action(Action::Key("moveDown:".into()));assert!(!process_inputs(&mut state,&mut window,&ui));
                assert_eq!(ui.consent_selection_revision(),revision,"editor navigation changed palette selection revision");
                poll_native(&mut state,&mut window,&ui);
                assert!(window.system.action.is_some());
                if change=="editor-cycle"{window.editing.cancel(window.editing.token().unwrap());}
            }
            "hide"=>{ui.action(Action::Key("cancelOperation:".into()));assert!(!process_inputs(&mut state,&mut window,&ui));}
            "mode"=>window.system.enter(Scope::Shortcuts),
            "query"=>{ui.query("Owned replacement");assert!(!process_inputs(&mut state,&mut window,&ui));}
            "config"=>{state.configuration.revision+=1;state.system.invalidate_if_revision(state.configuration.revision);}
            _=>panic!("unknown owned consent change"),
        }
        let selected=window.session.selected().map(|c|c.id.clone());
        let completed_before=window.system.completed.clone();
        let reads_before=fixture.reads.load(Ordering::Acquire);
        release.send(()).unwrap();
        completed.recv_timeout(Duration::from_secs(1)).unwrap();
        poll_native(&mut state,&mut window,&ui);
        assert!(window.system.action.is_none());
        if !matches!(change,"actions"|"editor"|"hide"|"mode"|"config") {
            let until=Instant::now()+Duration::from_secs(2);
            loop {
                tick(&mut window,&state,&ui);
                if fixture.reads.load(Ordering::Acquire)>reads_before && window.system.search.is_none()&&(change=="denied"||!window.system.snapshot.commands.is_empty()){break;}
                assert!(Instant::now()<until,"owned consent refresh did not complete");
                std::thread::sleep(Duration::from_millis(1));
            }
            assert!(fixture.reads.load(Ordering::Acquire)>reads_before,"consent completion did not run a fresh provider request");
            assert!(!window.system.refreshing,"consent preservation was not one-shot");
            if change=="denied" {
                assert!(window.system.snapshot.commands.is_empty());
                assert!(fixture.safari_denied.load(Ordering::Acquire));
                assert_eq!(window.session.rows().map(|c|c.id.as_str()).collect::<Vec<_>>(),["fixture.first","fixture.second"]);
                assert!(window.message.is_empty(),"explicit consent status must not replace the ordinary read result");
            }
            let reads=fixture.reads.load(Ordering::Acquire);
            poll_native(&mut state,&mut window,&ui);
            tick(&mut window,&state,&ui);
            assert_eq!(fixture.reads.load(Ordering::Acquire),reads,"completed consent refreshed twice");
            if change=="query"{
                assert_eq!(window.session.query,"Owned replacement");
                assert_eq!(window.system.snapshot.commands.len(),1);
                assert_eq!(window.system.snapshot.commands[0].title,"Owned replacement Tab");
            }
        } else {
            assert_eq!(fixture.reads.load(Ordering::Acquire),reads_before,"stale consent completion refreshed the provider");
            assert_eq!(window.system.completed,completed_before,"stale consent completion invalidated the actual completed search");
        }
        assert_eq!(*fixture.effects.lock().unwrap(),["consent"],"consent effect was repeated");
        (fixture.reads.load(Ordering::Acquire)>reads_before,selected,window.session.selected().map(|c|c.id.clone()))
    }
    #[test]
    fn held_denied_consent_refreshes_local_results_once_without_granting_access(){
        let (refreshed,_,_)=held_consent_completion("denied");
        assert!(refreshed);
    }
    #[test]
    fn held_consent_navigation_refreshes_without_resetting_the_selected_row(){
        let (refreshed,before,after)=held_consent_completion("arrow");
        assert_eq!(before.as_deref(),Some("fixture.second"));
        assert!(refreshed,"completed consent did not refresh after ordinary selection navigation");
        assert_eq!(after,before);
    }
    #[test]
    fn held_consent_default_and_closed_actions_choose_the_new_default_tab(){
        for change in ["default","actions-closed","actions-cycle","editor-cycle"]{
            let (refreshed,_,after)=held_consent_completion(change);
            assert!(refreshed,"{change}");
            assert!(after.is_some_and(|id|id.starts_with("system.tab:root.")),"{change}");
        }
    }
    #[test]
    fn held_consent_down_up_and_direct_selection_preserve_user_navigation(){
        for change in ["down-up","select"]{
            let (refreshed,before,after)=held_consent_completion(change);
            assert!(refreshed,"{change}");assert_eq!(after,before,"{change}");
        }
    }
    #[test]
    fn held_consent_current_query_is_refreshed_once_without_old_query_rows(){
        assert!(held_consent_completion("query").0);
    }
    #[test]
    fn held_consent_rejects_current_interaction_hide_mode_and_configuration_changes(){
        for change in ["actions","editor","hide","mode","config"]{
            let (refreshed,before,after)=held_consent_completion(change);
            assert!(!refreshed,"{change}");assert_eq!(after,before,"{change}");
        }
    }
    #[test]
    fn entering_another_mode_revokes_retained_action_ticket_without_reusing_authority() {
        let mut owner = Owner::default();
        let original = owner.scope_live.clone();
        assert!(original.load(Ordering::Acquire));
        owner.enter(Scope::Tabs);
        assert!(!original.load(Ordering::Acquire));
        let dedicated = owner.scope_live.clone();
        assert!(dedicated.load(Ordering::Acquire));
        owner.enter(Scope::MainTabs);
        assert!(!dedicated.load(Ordering::Acquire));
        assert!(!original.load(Ordering::Acquire));
        assert!(owner.scope_live.load(Ordering::Acquire));
    }
    #[test]
    fn native_and_cli_share_fractional_validated_budget_without_truncation() {
        let mut command: rustcast_core::Command = serde_json::from_value(json!({"id":"apple-shortcut.11111111-2222-3333-4444-555555555555","title":"Owned budget","kind":"script","target":"/usr/bin/shortcuts","arguments":["run","11111111-2222-3333-4444-555555555555"]})).unwrap();
        assert_eq!(action_budget(&command).unwrap(), Duration::from_secs(30));
        for value in [0.1, 0.9, 1.9, 300.] {
            command.timeout = Some(value);
            assert_eq!(action_budget(&command).unwrap().as_secs_f64(), value);
        }
        for value in [0., -1., f64::NAN, f64::INFINITY, 301.] {
            command.timeout = Some(value);
            assert!(action_budget(&command).is_err());
        }
    }

    fn ordinary_ax_search(scope: Scope, read_delay_ms: usize, identity_delay: bool) -> Result<Results,String> {
        let fixture=Arc::new(os::fixture::Fixture::default());
        fixture.delay_ms.store(read_delay_ms,Ordering::Release);
        if identity_delay {
            *fixture.before_identity.lock().unwrap()=Some(Box::new(|_|std::thread::sleep(Duration::from_millis(2100))));
        }
        let service=os::service::Service::new(fixture.clone());
        // Both ordinary runtime callers use this exact scope request budget;
        // CLI additionally retains min(context.deadline()). The provider owns
        // its separate original two-second traversal clock.
        let request=Ticket{deadline:Instant::now()+scope.budget(),live:Arc::new(||true)};
        let pending=service.search(scope,String::new(),fixture.previous(),request,Arc::new(||{})).unwrap();
        let until=Instant::now()+Duration::from_secs(10);
        let result=loop {
            if let Some(result)=pending.poll(){break result}
            assert!(Instant::now()<until,"owned caller-bound search did not terminate");
            std::thread::sleep(Duration::from_millis(2));
        };
        println!("owned {scope:?} request_budget={:?} identity_delay={identity_delay} rows={:?}",scope.budget(),result.as_ref().map(|rows|rows.commands.len()));
        assert!(fixture.effects.lock().unwrap().is_empty());
        result
    }
    #[test]
    fn ax_request_budget_windows_delivers_original_internal_partial_results() {
        let rows=ordinary_ax_search(Scope::Windows,250,false).expect("ordinary Window request must permit its private two-second partial result");
        assert_eq!(rows.commands.len(),1);
    }
    #[test]
    fn ax_request_budget_menus_delivers_original_internal_partial_results() {
        let rows=ordinary_ax_search(Scope::Menus,80,false).expect("ordinary Menu request must permit its private two-second partial result");
        assert_eq!(rows.commands.len(),1);
    }
    #[test]
    fn ax_request_budget_menu_identity_does_not_spend_private_traversal_clock() {
        let rows=ordinary_ax_search(Scope::Menus,0,true).expect("original Menu identity validation precedes its private two-second traversal clock");
        assert_eq!(rows.commands.len(),2);
    }
    struct BrowserCase {
        state: SharedState,
        window: WindowRuntime,
        ui: Arc<Connection>,
        fixture: Arc<os::fixture::Fixture>,
        platform: Arc<HeldConsent>,
        _temp: super::super::inline_tests::Temp,
    }
    impl BrowserCase {
        fn new() -> Self {
            let (temp,mut state,_,_,_)=super::super::inline_tests::setup();
            let fixture=Arc::new(os::fixture::Fixture::default());
            fixture.tabs.lock().unwrap().push(os::Tab{window:51,index:2,title:"Owned Other Tab".into(),url:"https://example.invalid/other".into()});
            let (entered,_)=mpsc::sync_channel(1);let (_,release)=mpsc::sync_channel(1);
            let platform=Arc::new(HeldConsent{browser_hold:Default::default(),denied:false,fixture:fixture.clone(),entered,release:Mutex::new(release)});
            state.system.service=os::service::Service::new(platform.clone());
            state.system.fixture=Some(fixture.clone());state.system.main_tabs_enabled=true;
            let (tx,_)=mpsc::sync_channel(8);let ui=Connection::new(tx,true);
            state.applications=vec![os::command("fixture.local".into(),"Owned Local Command".into(),None),os::command("fixture.second".into(),"Owned Second Command".into(),None)];
            let mut window=WindowRuntime::new(crate::session::Session::new(state.applications.clone()),Default::default(),&state);window.session.visible=true;
            Self{state,window,ui,fixture,platform,_temp:temp}
        }
        fn hold(&self)->(mpsc::Receiver<()>,mpsc::SyncSender<()>) {
            let (entered,wait)=mpsc::sync_channel(1);let (release,hold)=mpsc::sync_channel(1);
            assert!(self.platform.browser_hold.lock().unwrap().replace((entered,hold)).is_none());
            (wait,release)
        }
        fn query(&mut self,query:&str) {
            self.ui.query(query);assert!(!process_inputs(&mut self.state,&mut self.window,&self.ui));
            tick(&mut self.window,&self.state,&self.ui);
        }
        fn settle(&mut self) {
            let deadline=Instant::now()+Duration::from_secs(2);
            loop {
                tick(&mut self.window,&self.state,&self.ui);
                if self.window.system.search.is_none()&&self.window.system.completed.is_some(){break}
                assert!(Instant::now()<deadline,"owned main browser request did not settle");
                std::thread::sleep(Duration::from_millis(1));
            }
        }
    }
    #[test]
    fn browser_busy_main_cli_preserves_local_results_and_exactly_one_response() {
        use std::io::{BufRead,BufReader,Write};
        use std::os::unix::net::UnixStream;
        let mut case=BrowserCase::new();let (entered,release)=case.hold();case.query("Owned");
        entered.recv_timeout(Duration::from_secs(1)).unwrap();
        assert_eq!(case.window.session.rows().map(|c|c.id.as_str()).collect::<Vec<_>>(),["fixture.local","fixture.second"],"local rows must not wait for held Safari");
        let (events,requests)=mpsc::sync_channel(8);
        let server=crate::control::ControlServer::start(&case.state.configuration.paths.root,events,crate::control::Limits::default()).unwrap();
        let socket=case.state.configuration.paths.root.join("control.sock");
        let started=Instant::now();
        let client=std::thread::spawn(move||{
            let mut stream=UnixStream::connect(socket).unwrap();stream.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
            serde_json::to_writer(&mut stream,&json!({"op":"search","mode":"commands","query":"Owned Local"})).unwrap();stream.write_all(b"\n").unwrap();
            let mut reader=BufReader::new(stream);let mut line=String::new();reader.read_line(&mut line).unwrap();let response:Value=serde_json::from_str(&line).unwrap();
            line.clear();let extra=reader.read_line(&mut line).unwrap();(response,extra)
        });
        let request=match requests.recv_timeout(Duration::from_secs(1)).unwrap(){HostEvent::Request(request)=>request,_=>panic!("expected owned CLI request")};
        assert!(cli_search(&mut case.state,request));
        poll_cli(&mut case.state);
        let (response,extra)=client.join().unwrap();let elapsed=started.elapsed();
        release.send(()).unwrap();case.settle();poll_cli(&mut case.state);drop(server);
        assert_eq!(response["ok"],true,"busy optional provider must preserve CLI locals: {response}");
        assert_eq!(response["result"].as_array().unwrap().iter().map(|row|row["id"].as_str().unwrap()).collect::<Vec<_>>(),["fixture.local"]);
        assert!(elapsed<Duration::from_millis(900),"optional busy worker delayed CLI locals: {elapsed:?}");
        assert_eq!(extra,0,"late provider must not produce a second wire response");
        assert!(case.state.system.cli.is_empty());assert!(case.fixture.effects.lock().unwrap().is_empty());
    }

    #[test]
    fn browser_late_main_provider_preserves_navigation_without_retargeting_actions() {
        for interaction in ["navigate","actions"] {
            let mut case=BrowserCase::new();let (entered,release)=case.hold();case.query("Owned");entered.recv_timeout(Duration::from_secs(1)).unwrap();
            let action=if interaction=="navigate"{Action::Key("moveDown:".into())}else{Action::ToggleActions};
            case.ui.action(action);assert!(!process_inputs(&mut case.state,&mut case.window,&case.ui));
            let selected=case.window.session.selected().unwrap().id.clone();
            release.send(()).unwrap();case.settle();
            assert_eq!(case.window.session.selected().unwrap().id,selected,"late provider retargeted {interaction}");
            if interaction=="actions" {
                assert!(case.window.session.actions_open);
                assert!(case.window.system.snapshot.commands.is_empty());
                assert!(!case.window.session.rows().any(|row|row.id.starts_with("system.tab:")));
            } else {
                assert_eq!(selected,"fixture.second");
                assert_eq!(case.window.system.snapshot.commands.len(),2);
                assert!(case.window.session.rows().next().unwrap().id.starts_with("system.tab:root."));
            }
            assert!(case.fixture.effects.lock().unwrap().is_empty());
        }
    }
    #[test]
    fn browser_late_main_provider_never_crosses_hide_or_mode_change() {
        for hide in [true,false] {
            let mut case=BrowserCase::new();let (entered,release)=case.hold();case.query("Owned");entered.recv_timeout(Duration::from_secs(1)).unwrap();
            if hide {case.ui.action(Action::Key("cancelOperation:".into()));assert!(!process_inputs(&mut case.state,&mut case.window,&case.ui));assert!(!case.window.session.visible);}else{show_launcher_scope(&mut case.window,&case.ui,"snippets",Some(""),Instant::now()).unwrap();}
            release.send(()).unwrap();
            tick(&mut case.window,&case.state,&case.ui);
            if !hide {assert_eq!(case.window.session.launcher_scope,Some(crate::session::LauncherScope::Snippets));}
            assert!(!case.window.session.rows().any(|row|row.id.starts_with("system.tab:")));
            assert!(!case.window.system.snapshot.commands.iter().any(|row|row.id.starts_with("system.tab:")));
            if hide {assert!(case.window.system.snapshot.commands.is_empty());}
            assert!(case.fixture.effects.lock().unwrap().is_empty());
        }
    }
    #[test]
    fn browser_superseded_query_and_old_deadline_cannot_cancel_or_republish_latest_rows() {
        let mut case=BrowserCase::new();let (entered,release)=case.hold();case.query("Owned");entered.recv_timeout(Duration::from_secs(1)).unwrap();
        let mut old=case.window.system.search.take().unwrap();
        case.query("Other");release.send(()).unwrap();case.settle();
        assert_eq!(case.window.session.query,"Other");
        assert_eq!(case.window.system.snapshot.commands.len(),1);
        assert_eq!(case.window.system.snapshot.commands[0].title,"Owned Other Tab");
        let ids=case.window.session.rows().map(|c|c.id.clone()).collect::<Vec<_>>();
        let completed=case.window.system.completed.clone();
        old.pending.deadline=Instant::now();assert!(old.pending.poll().unwrap().is_err());
        tick(&mut case.window,&case.state,&case.ui);
        assert_eq!(case.window.system.completed,completed);
        assert_eq!(case.window.session.rows().map(|c|c.id.clone()).collect::<Vec<_>>(),ids);
        assert!(case.window.system.search.is_none());assert!(case.fixture.effects.lock().unwrap().is_empty());
    }
    #[test]
    fn browser_main_missing_denied_throttle_cache_and_explicit_refresh_never_prompt() {
        for missing in [false,true] {
            let mut case=BrowserCase::new();assert_eq!(case.fixture.reads.load(Ordering::Acquire),0,"startup must not read Safari");
            let apps=case.fixture.apps.lock().unwrap().clone();
            if missing {case.fixture.apps.lock().unwrap().clear();}else{case.fixture.safari_denied.store(true,Ordering::Release);}
            case.query("Owned");case.settle();
            assert_eq!(case.window.session.rows().map(|c|c.id.as_str()).collect::<Vec<_>>(),["fixture.local","fixture.second"]);
            assert!(case.window.message.is_empty());assert!(case.window.system.snapshot.commands.is_empty());
            let failed_reads=case.fixture.reads.load(Ordering::Acquire);assert!(failed_reads>0);
            case.query("Owned Local");case.settle();
            assert_eq!(case.fixture.reads.load(Ordering::Acquire),failed_reads,"failed reads were not throttled");
            assert_eq!(case.window.session.rows().next().unwrap().id,"fixture.local");
            case.fixture.safari_denied.store(false,Ordering::Release);*case.fixture.apps.lock().unwrap()=apps;
            case.state.system.service.refresh();case.window.system.refresh();case.query("Other");case.settle();
            assert_eq!(case.window.system.snapshot.commands.len(),1);let token=case.window.system.snapshot.commands[0].id.clone();
            let refreshed_reads=case.fixture.reads.load(Ordering::Acquire);assert!(refreshed_reads>failed_reads);
            case.query("Owned");case.settle();
            assert_eq!(case.fixture.reads.load(Ordering::Acquire),refreshed_reads+1,"cache hit may recheck browser identity, not rediscover metadata");
            assert!(case.window.system.snapshot.commands.iter().any(|row|row.id==token));
            assert_eq!(case.window.system.snapshot.commands.len(),2);assert!(case.fixture.effects.lock().unwrap().is_empty());
        }
    }
    #[test]
    fn browser_safari_send_crossing_the_real_deadline_stops_before_foreground_effects() {
        let fixture=Arc::new(os::fixture::Fixture::default());
        let provider=os::providers::Provider::new(fixture.clone(),false);
        let fresh=Ticket{deadline:Instant::now()+Duration::from_secs(2),live:Arc::new(||true)};
        let rows=provider.search(Scope::Tabs,"",None,&fresh).unwrap();
        let deadline=Instant::now()+Duration::from_secs(1);
        let reached=Arc::new(AtomicBool::new(false));let observed=reached.clone();
        *fixture.after_safari_select.lock().unwrap()=Some(Box::new(move |_|{
            assert!(Instant::now()<deadline,"initial reads/select did not reach the owned send before its deadline");
            observed.store(true,Ordering::Release);
            // A single bounded wait inside the owned send models its real
            // absolute deadline elapsing; no UI or original OS call occurs.
            std::thread::sleep(deadline.saturating_duration_since(Instant::now())+Duration::from_millis(1));
            assert!(Instant::now()>=deadline);
        }));
        let result=provider.execute(&rows.commands[0].id,None,&Ticket{deadline,live:Arc::new(||true)},&fixture.activate());
        assert!(reached.load(Ordering::Acquire),"owned select/send was not reached");
        assert!(Instant::now()>=deadline,"test did not cross the real deadline");
        assert_eq!(result,Err(os::CANCELLED.into()));
        assert_eq!(*fixture.effects.lock().unwrap(),["select-tab"],"expired post-send result reached foreground effects");
    }
    #[test]
    fn browser_expired_queued_activation_never_reaches_main_queue_eligible_or_effect() {
        let (client,main)=os::main_bridge::channel();let (wake,wait)=mpsc::channel();main.attach_wake(Arc::new(move||{let _=wake.send(());}));wait.recv().unwrap();
        let ticket=Ticket{deadline:Instant::now()+Duration::from_millis(30),live:Arc::new(||true)};
        let worker=std::thread::spawn(move||client.activate(os::Identity{pid:730001,uid:0,seconds:1,micros:0},&ticket));
        wait.recv_timeout(Duration::from_secs(1)).unwrap();std::thread::sleep(Duration::from_millis(60));
        main.drain_with(||panic!("expired activation reached foreground/eligibility"),|_,_|panic!("expired activation reached native effect"));
        assert!(worker.join().unwrap().is_err());
    }
    #[test]
    fn browser_dedicated_busy_and_expired_cli_requests_remain_errors() {
        use std::io::{BufRead,BufReader,Write};use std::os::unix::net::UnixStream;
        for expired in [false,true] {
            let mut case=BrowserCase::new();let (entered,release)=case.hold();
            let scope=if expired{Scope::MainTabs}else{Scope::Tabs};
            let pending=case.state.system.service.search(scope,"Owned".into(),None,Ticket{deadline:Instant::now()+Duration::from_secs(2),live:Arc::new(||true)},Arc::new(||{})).unwrap();entered.recv_timeout(Duration::from_secs(1)).unwrap();
            let (events,requests)=mpsc::sync_channel(8);let limits=crate::control::Limits{dispatch:if expired{Duration::from_millis(15)}else{Duration::from_secs(2)},..Default::default()};
            let server=crate::control::ControlServer::start(&case.state.configuration.paths.root,events,limits).unwrap();let socket=case.state.configuration.paths.root.join("control.sock");
            let client=std::thread::spawn(move||{let mut stream=UnixStream::connect(socket).unwrap();stream.set_read_timeout(Some(Duration::from_secs(2))).unwrap();serde_json::to_writer(&mut stream,&json!({"op":"search","mode":if expired{"commands"}else{"tabs"},"query":"Owned"})).unwrap();stream.write_all(b"\n").unwrap();let mut line=String::new();BufReader::new(stream).read_line(&mut line).unwrap();serde_json::from_str::<Value>(&line).unwrap()});
            let request=match requests.recv_timeout(Duration::from_secs(1)).unwrap(){HostEvent::Request(request)=>request,_=>panic!("expected owned request")};
            if expired{std::thread::sleep(Duration::from_millis(25));assert!(!request.context.is_live());}
            assert!(cli_search(&mut case.state,request));let response=client.join().unwrap();release.send(()).unwrap();drop(pending);drop(server);
            assert_eq!(response["ok"],false,"dedicated/expired error was masked: {response}");
            assert!(case.state.system.cli.is_empty());assert!(case.fixture.effects.lock().unwrap().is_empty());
        }
    }

    #[test]
    fn browser_eight_pending_cli_requests_do_not_block_live_local_root_results() {
        use std::io::{BufRead,BufReader,Write};use std::os::unix::net::UnixStream;
        fn client(socket:std::path::PathBuf,value:Value)->std::thread::JoinHandle<Value>{
            std::thread::spawn(move||{let mut stream=UnixStream::connect(socket).unwrap();stream.set_read_timeout(Some(Duration::from_secs(3))).unwrap();serde_json::to_writer(&mut stream,&value).unwrap();stream.write_all(b"\n").unwrap();let mut line=String::new();BufReader::new(stream).read_line(&mut line).unwrap();serde_json::from_str(&line).unwrap()})
        }
        let mut case=BrowserCase::new();let(events,requests)=mpsc::sync_channel(32);case.state.events=events.clone();
        let server=crate::control::ControlServer::start(&case.state.configuration.paths.root,events,Default::default()).unwrap();let socket=case.state.configuration.paths.root.join("control.sock");
        let mut clients=vec![];let mut held=None;
        for index in 0..8 {
            if index==7{case.state.system.service.refresh();held=Some(case.hold());}
            clients.push(client(socket.clone(),json!({"op":"search","mode":"commands","query":"Owned"})));
            let request=loop{match requests.recv_timeout(Duration::from_secs(1)).unwrap(){HostEvent::Request(request)=>break request,HostEvent::UiWake=>(),_=>panic!("unexpected owned request event")}};
            assert!(cli_search(&mut case.state,request));
            assert_eq!(case.state.system.cli.len(),index+1,"each retained CLI record must come from a real admitted request");
            if index<7 {
                assert!(matches!(requests.recv_timeout(Duration::from_secs(1)).unwrap(),HostEvent::UiWake));
                // The worker's actual wake follows result publication. Give
                // its immediately following stack guard time to exit before
                // admitting the next request; no retry or synthetic Cli row.
                std::thread::sleep(Duration::from_millis(2));
            }
        }
        let (entered,release)=held.unwrap();entered.recv_timeout(Duration::from_secs(1)).unwrap();
        let start=Instant::now();let last=client(socket,json!({"op":"search","mode":"commands","query":"Owned","limit":1}));
        let request=match requests.recv_timeout(Duration::from_secs(1)).unwrap(){HostEvent::Request(request)=>request,_=>panic!("expected ninth real CLI request")};
        assert!(cli_search(&mut case.state,request));let response=last.join().unwrap();let elapsed=start.elapsed();
        release.send(()).unwrap();
        let deadline=Instant::now()+Duration::from_secs(2);
        while !case.state.system.cli.is_empty(){poll_cli(&mut case.state);assert!(Instant::now()<deadline);std::thread::sleep(Duration::from_millis(1));}
        for client in clients{assert_eq!(client.join().unwrap()["ok"],true);}
        drop(server);
        assert_eq!(response["ok"],true,"optional queue capacity must not reject live local root search: {response}");
        assert_eq!(response["result"].as_array().unwrap().len(),1,"fallback must retain caller limit");
        assert_eq!(response["result"][0]["id"],"fixture.local");
        assert!(elapsed<Duration::from_millis(900));assert!(case.fixture.effects.lock().unwrap().is_empty());
    }

}
