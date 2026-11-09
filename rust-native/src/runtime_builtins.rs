//! Native and CLI builtin composition over the existing owner/effect lanes.
use super::*;
pub(crate) fn known(target: &str) -> bool {
    matches!(
        target,
        "settings" | "open-config" | "reload-config" | "system-monitor" | "lock-screen"
            | "clipboard" | "snippets" | "quicklinks" | "create-snippet" | "create-quicklink"
    ) || builtin_scope(target).is_some()
}
fn navigation_target(target: &str) -> bool {
    matches!(target, "clipboard" | "snippets" | "quicklinks" | "create-snippet" | "create-quicklink")
}
fn navigate(state: &SharedState, w: &mut WindowRuntime, ui: &Arc<Connection>, target: &str) -> Result<(), String> {
    match target {
        "clipboard" => show_clipboard(state, w, ui, Instant::now()),
        "snippets" | "quicklinks" => show_launcher_scope(w, ui, target, None, Instant::now()),
        "create-snippet" | "create-quicklink" => {
            if !w.session.visible { w.show(Some(""), Instant::now(), ui)?; }
            w.session.actions_open = false;
            w.message.clear();
            w.retire_creation();
            w.editing.cancel_all();
            w.editing.create(&state.configuration, if target == "create-snippet" { "snippet" } else { "url" }, None)?;
            if target == "create-quicklink" {
                start_creation(state, w, ui, None, ui.latest_dismissal(), Instant::now() + Duration::from_secs(5));
            }
            w.stamp = None;
            Ok(())
        }
        _ => Err("Unknown navigation command.".into()),
    }
}
fn command(state: &SharedState, query: &str) -> Option<rustcast_core::Command> {
    let key = command_key(query);
    crate::management_commands::catalog(&state.configuration.current().commands)
        .into_iter().find(|c| name_matches(c, &key))
}
fn prepare(
    state: &SharedState,
    c: &rustcast_core::Command,
) -> Result<rustcast_core::Command, String> {
    let resolved = command(state, &c.id)
        .filter(|candidate| execution_identity(c, candidate))
        .ok_or("Selection changed. Try again.")?;
    if resolved.enabled == Some(false) {
        return Err("Command is disabled.".into());
    }
    Ok(resolved)
}
fn effect(
    state: &SharedState,
    c: &rustcast_core::Command,
    ticket: Ticket,
) -> Result<Pending<()>, String> {
    let root = state.configuration.paths.root.clone();
    let target = c.target.clone();
    let file = state.configuration.file.clone();
    let services = state.services.as_ref().map(|s| s.system.clone());
    #[cfg(any(test, feature = "test-tools"))]
    let fixture = state.system.fixture.is_some();
    #[cfg(not(any(test, feature = "test-tools")))]
    let fixture = false;
    state
        .system
        .service
        .effect(ticket, wake(state), move |ticket| {
            if target == "reload-config" {
                return crate::builtin_effects::reload(&root, &ticket);
            }
            if target == "lock-screen" && !fixture && crate::system_actions::legacy_lock(&ticket)? {
                return Ok(());
            }
            let value = match target.as_str() {
                "open-config" => crate::system_actions::Effect::Configuration(file),
                "lock-screen" => crate::system_actions::Effect::Lock,
                _ => return Err("Unknown builtin effect.".into()),
            };
            services
                .ok_or("Native system service unavailable.")?
                .effect(value, &ticket)
        })
}
pub(crate) fn native(
    state: &mut SharedState,
    w: &mut WindowRuntime,
    ui: &Arc<Connection>,
    c: &rustcast_core::Command,
    values: &Value,
    input_token: Option<u64>,
) -> Result<(), String> {
    let command = prepare(state, c)?;
    crate::builtin_effects::validate(&command, values)?;
    if w.system.action.is_some() {
        return Err("A system action is already running.".into());
    }
    if navigation_target(&command.target) {
        let record = record_start(state, &command, w.native_source, values);
        if let Some(token) = input_token { w.editing.cancel(token); }
        let result = navigate(state, w, ui, &command.target);
        let mut response=match &result{Ok(())=>json!({"ok":true}),Err(error)=>json!({"ok":false,"error":error})};
        let accounting = record_finish(state, record, &mut response, true);
        return result.and_then(|_| accounting.map_err(|error| format!("Action completed, but usage could not be saved: {error}")));
    }
    if command.target == "system-monitor" {
        if let Some(token) = input_token {
            w.editing.cancel(token);
        }
        return open_monitor(state, w, &command);
    }
    if command.target == "settings" || builtin_scope(&command.target).is_some() {
        let record = record_start(state, &command, w.native_source, values);
        let result = (|| -> Result<(), String> {
            if let Some(token) = input_token {
                w.editing.cancel(token);
            }
            if command.target == "settings" {
                let peer = ui.peer().ok_or("Settings window is unavailable.")?;
                peer.action(Action::Show);
                w.session.visible = false;
            } else {
                w.show(Some(""), Instant::now(), ui)?;
                w.system.enter(builtin_scope(&command.target).unwrap());
                if command.target == "search-safari-tabs" {
                    consent(state, w, ui)?;
                }
            }
            Ok(())
        })();
        let mut response=match &result{Ok(())=>json!({"ok":true}),Err(error)=>json!({"ok":false,"error":error})};
        let _ = record_finish(state, record, &mut response, true);
        return result;
    }
    let epoch = ui.latest_epoch();
    let intent = ui.latest_intent();
    let dismissal = ui.latest_dismissal();
    let revision = state.configuration.revision;
    let allow_revision = command.target == "reload-config";
    let config = state.system.pin(revision);
    let scope_live = w.system.scope_live.clone();
    let ticket_scope_live = scope_live.clone();
    let weak = Arc::downgrade(ui);
    let ticket = Ticket {
        deadline: Instant::now() + Duration::from_secs(5),
        live: Arc::new(move || {
            ticket_scope_live.load(Ordering::Acquire)
                && (allow_revision || config.load(Ordering::Acquire) == revision)
                && weak.upgrade().is_some_and(|ui| {
                    !ui.stop_requested()
                        && ui.latest_epoch() == epoch
                        && ui.latest_intent() == intent
                        && ui.latest_dismissal() == dismissal
                        && input_token.is_none_or(|token| !ui.editor_cancelled(token))
                })
        }),
    };
    let record = record_start(state, &command, w.native_source, values);
    let pending = match effect(state, &command, ticket) {
        Ok(p) => p,
        Err(e) => {
            let _ = record_finish(state, record, &mut json!({"ok":false,"error":e}), true);
            return Err(e);
        }
    };
    w.system.action = Some(ActionJob {
        scope_live,
        pending: ActionPending::Action(pending),
        epoch,
        intent,
        dismissal,
        revision,
        consent: false,
            consent_selection_revision: 0,
        record: Some(record),
        hide_key: None,
        hidden: Arc::new(AtomicBool::new(false)),
        process_title: None,
        allow_revision,
        keep_palette: allow_revision,
        input_token,
    });
    w.session.actions_open = false;
    w.message.clear();
    w.stamp = None;
    Ok(())
}
pub(crate) fn cli(
    state: &mut SharedState,
    w: &mut WindowRuntime,
    ui: Option<&Arc<Connection>>,
    request: crate::control::Envelope,
) -> Result<(), crate::control::Envelope> {
    let Some(command) = command(state, request.value["id"].as_str().unwrap_or(""))
        .filter(|c| c.kind == "builtin" && known(&c.target))
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
    if let Err(error) = crate::builtin_effects::validate(&command, &values) {
        let _ = request.respond(json!({"ok":false,"error":error}));
        return Ok(());
    }
    if matches!(
        command.target.as_str(),
        "reload-config" | "lock-screen" | "open-config"
    ) {
        if command.target == "open-config" && ui.is_none() {
            let _ = request.respond(
                json!({"ok":false,"error":"Open Configuration requires the app interface."}),
            );
            return Ok(());
        }
        if state.system.cli.len() >= 8 {
            let _ =
                request.respond(json!({"ok":false,"error":"System action is busy. Try again."}));
            return Ok(());
        }
        let revision = state.configuration.revision;
        let config = state.system.pin(revision);
        let context = request.context.clone();
        let allow_revision = command.target == "reload-config";
        let ticket = Ticket {
            deadline: context
                .deadline()
                .min(Instant::now() + Duration::from_secs(5)),
            live: Arc::new(move || {
                context.is_live() && (allow_revision || config.load(Ordering::Acquire) == revision)
            }),
        };
        let outcome = match command.target.as_str() {
            "reload-config" => "reloaded",
            "open-config" => "opened",
            _ => "dispatched",
        };
        let record = record_start(state, &command, "cli", &values);
        match effect(state, &command, ticket) {
            Ok(pending) => state.system.cli.push(Cli {
                request,
                pending: CliPending::Builtin(pending, outcome),
                scope: Scope::MainTabs,
                revision,
                record: Some(record),
            }),
            Err(error) => {
                let mut response=json!({"ok":false,"error":error});
                let _ = record_finish(state, record, &mut response, true);
                let _ = request.respond(response);
            }
        }
        return Ok(());
    }
    let record = record_start(state, &command, "cli", &values);
    let result = (|| -> Result<(), String> {
        let ui = ui.ok_or_else(|| {
            if command.target == "system-monitor" {
                "The monitor window is unavailable in headless mode. Use ruycast monitor for data."
            } else {
                "The palette is unavailable in headless mode."
            }
        })?;
        match command.target.as_str() {
            target if navigation_target(target) => navigate(state, w, ui, target)?,
            "system-monitor" => {
                state.system.monitor.open()?;
                w.session.visible = false;
            }
            "settings" => {
                ui.peer()
                    .ok_or("Settings window is unavailable.")?
                    .action(Action::Show);
                w.session.visible = false;
            }
            _ => {
                w.show(Some(""), Instant::now(), ui)?;
                w.system.enter(builtin_scope(&command.target).unwrap());
            }
        }
        Ok(())
    })();
    let mut response=match &result{Ok(())=>json!({"ok":true}),Err(error)=>json!({"ok":false,"error":error})};
    let accounting = record_finish(state, record, &mut response, true);
    let reply = match result {
        Err(_) => response,
        Ok(()) => match accounting {
            Ok(()) => json!({"ok":true,"result":{"state":"opened"}}),
            Err(error) => json!({"ok":false,"error":format!("Action completed, but usage could not be saved: {error}"),"actionCompleted":true}),
        }
    };
    let _ = request.respond(
        reply,
    );
    Ok(())
}
