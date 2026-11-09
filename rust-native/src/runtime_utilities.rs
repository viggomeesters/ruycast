//! Utilities composition. All profile/Keychain I/O runs in bounded workers;
//! actual clipboard/desktop effects reuse the existing main-owned paste tickets.
use super::*;
use crate::{
    utilities::Effect,
    utility_jobs::{Jobs, Operation, Output, Store},
    utility_storage::{Authority, NativeStorage},
};
use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicBool, AtomicU8, Ordering},
        Mutex,
    },
};

enum QueryContext {
    Cli {
        request: crate::control::Envelope,
        mode: Option<String>,
    },
    Native {
        revoked: Arc<AtomicBool>,
        mode: String,
        epoch: u64,
        intent: u64,
        dismissal: u64,
        revision: u64,
    },
}
pub(super) struct State {
    pub store: Store,
    jobs: Jobs<QueryContext>,
    known: HashMap<String, rustcast_core::Command>,
    kinds: HashMap<String, String>,
    pub changes: HashMap<String, Arc<AtomicU8>>,
}
impl State {
    #[cfg(feature = "test-tools")]
    pub(super) fn owned_pending(&self) -> Value {
        self.jobs.owned_pending()
    }
    pub fn new(
        paths: &ProfilePaths,
        fixture: bool,
        lease: Option<crate::control::ProfileLease>,
        wake: mpsc::SyncSender<HostEvent>,
    ) -> Result<Self, String> {
        let mut storage = if fixture {
            NativeStorage::fixture(paths.root.clone())?
        } else {
            NativeStorage::production(paths.root.clone())?
        };
        if let Some(lease) = lease {
            storage.retain_profile(lease);
        }
        let store = Arc::new(Mutex::new(crate::utilities::Utilities::new(storage)));
        Ok(Self {
            jobs: Jobs::new(store.clone(), wake),
            store,
            known: Default::default(),
            kinds: Default::default(),
            changes: Default::default(),
        })
    }
    pub fn command(&self, id: &str) -> Option<&rustcast_core::Command> {
        self.known.values().find(|c| command_key(&c.id) == id)
    }
    pub fn next_wake(&self) -> Option<Instant> {
        self.jobs.next_wake()
    }
    fn remember(&mut self, mode: &str, rows: &[rustcast_core::Command]) {
        let kind = kind(mode).unwrap_or("");
        self.known
            .retain(|id, _| self.kinds.get(id).is_none_or(|old| old != kind));
        self.kinds.retain(|_, old| old != kind);
        // The domain owns the exact expiring token map. Keep only bounded
        // metadata here, never the text/password values behind a token.
        if self.known.len() + rows.len() > 250 {
            self.known.clear();
            self.kinds.clear();
        }
        for c in rows {
            self.kinds.insert(c.id.clone(), kind.into());
            self.known.insert(c.id.clone(), c.clone());
        }
    }
}
#[derive(Default)]
pub(super) struct Window {
    pub revoked: Arc<AtomicBool>,
    pub mode: Option<String>,
    stamp: Option<(String, String, u64, u64, u64)>,
    pub busy: bool,
    ready: Option<Vec<rustcast_core::Command>>,
}
impl Drop for Window {
    fn drop(&mut self) {
        self.revoked.store(true, Ordering::Release);
    }
}
pub(super) fn kind(mode: &str) -> Option<&'static str> {
    match mode.strip_prefix("utility-").unwrap_or(mode) {
        "text-history" => Some("text"),
        "password-history" => Some("password"),
        "username-history" => Some("username"),
        _ => None,
    }
}
pub(super) fn recognizes(command: &rustcast_core::Command) -> bool {
    command.kind == "builtin"
        && (command.target.starts_with("utility-")
            || command.target.starts_with("utility.history:"))
}
pub(super) fn show(
    state: &mut SharedState,
    w: &mut WindowRuntime,
    ui: &Arc<Connection>,
    mode: &str,
    query: Option<&str>,
    received: Instant,
) -> Result<(), String> {
    if kind(mode).is_none() {
        return Err("Unknown Utility history.".into());
    }
    w.show(query.or(Some("")), received, ui)?;
    w.utility.mode = Some(format!("utility-{}-history", kind(mode).unwrap()));
    w.utility.stamp = None;
    refresh(state, w, ui);
    Ok(())
}
pub(super) fn show_command(
    state: &mut SharedState,
    w: &mut WindowRuntime,
    ui: &Arc<Connection>,
    command: &rustcast_core::Command,
    received: Instant,
    values: &Value,
) -> (Result<(), String>, Option<String>) {
    let source = w.native_source;
    let started = Instant::now();
    let trace = uuid();
    let span = crate::analytics::Span::new(state.analytics.client(), &command.id, source);
    let logged_id = crate::activity::Activity::log_id(&command.id, values, &state.configuration.paths.home);
    state.activity.record(&trace, &logged_id, source, None);
    let mut result = show(state, w, ui, &command.target, None, received);
    if result.is_ok() {
        if let Err(error) = state.activity.success(&command.id) {
            result = Err(format!(
                "History opened, but usage could not be saved: {error}"
            ));
        }
        if source == "launcher" {
            if let Err(error) = state
                .ranking
                .record_success(&command.id, crate::activity::now())
            {
                result = Err(format!(
                    "History opened, but search ranking could not be saved: {error}"
                ));
            }
        }
    }
    let success = result.is_ok();
    let response=match &result {Ok(())=>json!({"ok":true}),Err(error)=>json!({"ok":false,"error":error})};
    state.activity.record_response(
        &trace,
        &logged_id,
        source,
        started.elapsed().as_secs_f64() * 1000.,
        &response,
        (&log_secrets(values), &state.configuration.paths.home),
    );
    if let Some(span) = span {
        span.finish(
            success,
            crate::analytics::failure_code(result.as_ref().err().map(String::as_str).unwrap_or("")),
        );
    }
    let diagnostic = result.is_err().then_some(trace);
    (result, diagnostic)
}
pub(super) fn refresh(state: &mut SharedState, w: &mut WindowRuntime, ui: &Arc<Connection>) {
    if w.editing.token().is_none() && !w.session.actions_open {
        if let Some(rows) = w.utility.ready.take() {
            w.session.set_external_results(rows, true);
            w.stamp = None;
        }
    }
    let Some(mode) = w
        .utility
        .mode
        .clone()
        .filter(|_| w.session.visible && w.session.library.is_none())
    else {
        return;
    };
    let stamp = (
        mode.clone(),
        w.session.query.clone(),
        w.epoch,
        ui.latest_dismissal(),
        state.configuration.revision,
    );
    if w.utility.stamp.as_ref() == Some(&stamp) {
        return;
    }
    w.utility.stamp = Some(stamp);
    w.utility.busy = true;
    w.session.set_external_results(vec![], false);
    w.stamp = None;
    let epoch = w.epoch;
    let intent = ui.latest_intent();
    let dismissal = ui.latest_dismissal();
    let revision = state.configuration.revision;
    let weak = Arc::downgrade(ui);
    let revoked = w.utility.revoked.clone();
    let query_revoked = revoked.clone();
    let live: crate::execution::Live = Arc::new(move || {
        !revoked.load(Ordering::Acquire)
            && weak.upgrade().is_some_and(|ui| {
                !ui.stop_requested()
                    && ui.latest_epoch() == epoch
                    && ui.latest_dismissal() == dismissal
            })
    });
    let context = QueryContext::Native {
        revoked: query_revoked,
        mode: mode.clone(),
        epoch,
        intent,
        dismissal,
        revision,
    };
    if let Err((error, _)) = state.utility.jobs.start(
        Operation::Search {
            mode,
            query: w.session.query.clone(),
        },
        live,
        Instant::now() + Duration::from_secs(5),
        context,
    ) {
        w.utility.busy = false;
        w.utility.stamp = None;
        w.message = error;
    }
}
pub(super) fn request(
    state: &mut SharedState,
    request: crate::control::Envelope,
) -> Result<(), crate::control::Envelope> {
    let operation = match request.value["op"].as_str() {
        Some("search") => {
            let mode = request.value["mode"]
                .as_str()
                .or(request.value["scope"].as_str())
                .unwrap_or("commands");
            if kind(mode).is_none() {
                return Err(request);
            }
            Operation::Search {
                mode: mode.into(),
                query: request.value["query"].as_str().unwrap_or("").into(),
            }
        }
        Some("utilities.import") => {
            if !request.value["profile"].is_object() {
                let _ = request.respond(json!({"ok":false,"error":"Missing utilities profile."}));
                return Ok(());
            }
            Operation::Import(
                request.value["profile"].clone(),
                request.context.utility_publication(false),
            )
        }
        _ => return Err(request),
    };
    let mode = match &operation {
        Operation::Search { mode, .. } => Some(mode.clone()),
        _ => None,
    };
    let context = request.context.clone();
    let deadline = context.deadline();
    let live = Arc::new(move || context.is_live());
    if let Err((error, QueryContext::Cli { request, .. })) = state.utility.jobs.start(
        operation,
        live,
        deadline,
        QueryContext::Cli { request, mode },
    ) {
        let _ = request.respond(json!({"ok":false,"error":error}));
    }
    Ok(())
}
pub(super) fn poll(state: &mut SharedState, w: &mut WindowRuntime, ui: Option<&Arc<Connection>>) {
    for (context, result) in state.utility.jobs.poll() {
        match context {
            QueryContext::Cli { request, mode } => {
                let response = match result {
                    Ok(Output::Rows(rows)) => {
                        if let Some(mode) = mode {
                            state.utility.remember(&mode, &rows);
                        }
                        json!({"ok":true,"result":rows})
                    }
                    Ok(Output::Imported(result, warning)) => {
                        json!({"ok":true,"result":result,"durabilityWarning":warning})
                    }
                    Err(error) => json!({"ok":false,"error":error}),
                };
                let _ = request.respond(response);
            }
            QueryContext::Native {
                revoked,
                mode,
                epoch,
                intent,
                dismissal,
                revision,
            } => {
                let Some(ui) = ui else {
                    continue;
                };
                if revoked.load(Ordering::Acquire)
                    || !Arc::ptr_eq(&revoked, &w.utility.revoked)
                    || w.utility.mode.as_deref() != Some(&mode)
                    || w.epoch != epoch
                    || ui.latest_epoch() != epoch
                    || ui.latest_dismissal() != dismissal
                    || !w.session.visible
                    || state.configuration.revision != revision
                {
                    continue;
                }
                w.utility.busy = false;
                w.stamp = None;
                match result {
                    Ok(Output::Rows(rows)) => {
                        state.utility.remember(&mode, &rows);
                        if !w.session.actions_open && w.editing.token().is_none() {
                            w.session
                                .set_external_results(rows, ui.latest_intent() != intent);
                            w.message.clear();
                        } else {
                            w.utility.ready = Some(rows);
                        }
                    }
                    Err(error) => {
                        w.session.set_external_results(vec![], false);
                        w.message = error;
                    }
                    _ => {}
                }
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn start(
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
    if command.enabled == Some(false) {
        return Err("Command is disabled.".into());
    }
    crate::management_commands::validate_input(&command, &values)?;
    let services = match state.services.clone() {
        Some(services) => services,
        None => {
            let error = "Main-thread service is unavailable.";
            if let ExecutionDestination::Cli(request) = destination {
                let _ = request.respond(json!({"ok":false,"error":error}));
            }
            return Err(error.into());
        }
    };
    let store = state.utility.store.clone();
    let trace = uuid();
    let source = destination.source();
    let permit = crate::main_services::Permit::default();
    let revoked = permit.live();
    let mut authority = Authority::read_write(live.clone(), Arc::new(|| Ok(())));
    if let ExecutionDestination::Cli(request) = &destination {
        authority.changed = request.context.utility_publication(true);
    }
    let dispatch = match &destination {
        ExecutionDestination::Cli(request) => request.context.paste_dispatch(),
        _ => Arc::new(crate::paste::Dispatch::default()),
    };
    let output_dispatch = dispatch.clone();
    let hide = hide_origin
        .as_ref()
        .map(|o| o.slot.clone())
        .unwrap_or_default();
    let has_hide = hide_origin.is_some();
    let grant = owner_native_grant(
        state.events.clone(),
        trace.clone(),
        live.clone(),
        deadline,
        2,
    );
    authority.set_grant(grant);
    let changed = authority.changed.clone();
    let logged_id =
        crate::activity::Activity::log_id(&command.id, &values, &state.configuration.paths.home);
    let context = ExecutionContext {
        capture_result: None,
        initial_result: None,
        analytics: crate::analytics::Span::new(
            state.analytics.client(),
            analytics_id(&command),
            source,
        ),
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
    let events = state.events.clone();
    let request_trace = trace.clone();
    match state.execution.admit_snippet(
        source == "cli",
        live,
        deadline,
        dispatch,
        context,
        move |live| {
            let result = (|| -> Result<_, String> {
                if !live() {
                    return Err("Utilities request expired or was cancelled.".into());
                }
                let mut store = store.try_lock().map_err(|_| {
                    "Utilities are busy. Try again after the current operation finishes."
                })?;
                store
                    .storage_mut()
                    .set_authority(Some(authority.with_live(live.clone())));
                let result = store.execute(&command.target, values["input"].as_str());
                let warning = store.take_publication_warning();
                store.storage_mut().set_authority(None);
                result.map(|effect| (effect, warning))
            })();
            let (effect, warning) = match result {
                Ok(value) => value,
                Err(error) => return crate::paste::Outcome::failure(error, false),
            };
            let (mut outcome, notice, manual_hide) = match effect {
                Effect::Saved(notice) => (
                    crate::paste::Outcome {
                        response: json!({"ok":true}),
                        committed: true,
                    },
                    notice,
                    true,
                ),
                Effect::Paste(text)
                | Effect::Copy {
                    text,
                    secret: false,
                } => {
                    let copy_only = command.target != "utility-paste-text";
                    let prepared = crate::paste::Prepared::utility(text, false, copy_only);
                    let grant_events = events.clone();
                    let grant_trace = request_trace.clone();
                    let grant_live = live.clone();
                    let result = services.paste().execute(
                        prepared,
                        hide.clone(),
                        live.clone(),
                        deadline,
                        revoked.clone(),
                        output_dispatch.clone(),
                        move |hide| {
                            owner_native_grant(
                                grant_events.clone(),
                                grant_trace.clone(),
                                grant_live.clone(),
                                deadline,
                                if hide { 4 } else { 3 },
                            )()
                        },
                    );
                    (result, if copy_only { "Copied" } else { "Done" }, copy_only)
                }
                Effect::Copy { text, secret: true } => {
                    let prepared = crate::paste::Prepared::utility(text, true, true);
                    let grant_events = events.clone();
                    let grant_trace = request_trace.clone();
                    let grant_live = live.clone();
                    let result = services.paste().execute(
                        prepared,
                        hide.clone(),
                        live.clone(),
                        deadline,
                        revoked.clone(),
                        output_dispatch.clone(),
                        move |hide| {
                            owner_native_grant(
                                grant_events.clone(),
                                grant_trace.clone(),
                                grant_live.clone(),
                                deadline,
                                if hide { 4 } else { 3 },
                            )()
                        },
                    );
                    (result, "Copied", true)
                }
            };
            if outcome.response["ok"] == true && manual_hide && has_hide {
                let hidden = (|| -> Result<(), String> {
                    owner_native_grant(
                        events.clone(),
                        request_trace.clone(),
                        live.clone(),
                        deadline,
                        3,
                    )()?;
                    owner_native_grant(events, request_trace, live.clone(), deadline, 4)()?;
                    let token = hide
                        .lock()
                        .unwrap_or_else(|p| p.into_inner())
                        .clone()
                        .ok_or("Utility window changed.")?;
                    services.hide_utility(token, deadline, live, revoked, output_dispatch.clone())
                })();
                if let Err(error) = hidden {
                    outcome.response = json!({"ok":false,"error":error,"actionCompleted":true});
                }
            }
            outcome.response["utilityNotice"] = json!(notice);
            if let Some(warning) = warning {
                outcome.response["durabilityWarning"] = json!(warning);
            }
            output_dispatch.complete(&outcome);
            outcome
        },
    ) {
        Ok(_) => {
            state.utility.changes.insert(trace.clone(), changed);
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

pub(super) fn start_native(
    state: &mut SharedState,
    w: &mut WindowRuntime,
    ui: &Arc<Connection>,
    dismissal: u64,
    command: rustcast_core::Command,
    values: Value,
    input_token: Option<u64>,
) -> Result<(), String> {
    if command.enabled == Some(false) {
        return Err("Command is disabled.".into());
    }
    crate::management_commands::validate_input(&command, &values)?;
    if kind(&command.target).is_some() {
        return show_command(state, w, ui, &command, Instant::now(), &values).0;
    }
    let serial = w
        .execution_serial
        .checked_add(1)
        .ok_or("Execution sequence exhausted.")?;
    let epoch = w.epoch;
    let intent = w.input_intent;
    let weak = Arc::downgrade(ui);
    let hide = SnippetHideOrigin::capture(ui, Some(serial));
    let check = hide.clone();
    let revoked = w.utility.revoked.clone();
    let live: crate::execution::Live = Arc::new(move || {
        !revoked.load(Ordering::Acquire)
            && check.live()
            && weak
                .upgrade()
                .is_some_and(|ui| input_token.is_none_or(|t| !ui.editor_cancelled(t)))
    });
    let origin = if command.target.starts_with("utility.history:") {
        ExecutionOrigin::Utility
    } else {
        ExecutionOrigin::Saved
    };
    start(
        state,
        command,
        values,
        ExecutionDestination::Native {
            source: w.native_source,
            ui: Arc::downgrade(ui),
            epoch,
            dismissal,
            intent,
            serial,
            input_token,
            result_origin: None,
        },
        origin,
        None,
        live,
        native_open_deadline(state),
        Some(hide),
    )?;
    w.execution_serial = serial;
    w.execution_effect = None;
    w.message.clear();
    w.session.actions_open = false;
    Ok(())
}
