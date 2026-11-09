use super::*;

// A saved capture may request one native open. Reuse the exact workspace
// dispatch, owner grant, hide acknowledgement and request permit; never rerun
// the script, transfer its saved truth to a newer selection, or open for CLI.
pub(super) fn start_capture_open(
    state: &mut SharedState,
    window: &WindowRuntime,
    mut context: ExecutionContext,
    saved: Value,
    request: crate::workspace::OpenRequest,
) -> Result<(), Box<ExecutionContext>> {
    let prepare = (|| -> Result<_, String> {
        let ExecutionDestination::Native {
            ui,
            epoch,
            dismissal,
            intent,
            serial,
            input_token,
            result_origin,
            ..
        } = &context.destination
        else {
            return Err("Capture opener is unavailable for CLI.".into());
        };
        let ui = ui
            .upgrade()
            .ok_or("Capture window is no longer available.")?;
        if result_origin.is_some()
            || state.configuration.last_error.is_some()
            || !origin_valid(state, &context)
            || ui.stop_requested()
            || ui.latest_epoch() != *epoch
            || ui.latest_dismissal() != *dismissal
            || ui.latest_intent() != *intent
            || window.epoch != *epoch
            || window.execution_serial != *serial
            || window.session.actions_open
            || input_token
                .is_some_and(|t| ui.editor_cancelled(t) || window.editing.token() != Some(t))
            || (input_token.is_none()
                && context.source() != "hotkey"
                && window
                    .session
                    .selected()
                    .is_none_or(|c| c.id != context.command.id))
        {
            return Err("Capture selection changed before opening.".into());
        }
        let services = state
            .services
            .clone()
            .ok_or("Main-thread service is unavailable.")?;
        let key = crate::connected::HideKey {
            trace: context.trace.clone(),
            epoch: *epoch,
            dismissal: *dismissal,
            intent: *intent,
            serial: *serial,
        };
        let token = ui.arm_open_hide(key.clone())?;
        let weak = Arc::downgrade(&ui);
        let input = *input_token;
        let live: crate::execution::Live = Arc::new(move || {
            weak.upgrade().is_some_and(|ui| {
                ui.open_hide_live(&key) && input.is_none_or(|t| !ui.editor_cancelled(t))
            })
        });
        Ok((ui, services, token, live))
    })();
    let (ui, services, token, live) = match prepare {
        Ok(value) => value,
        Err(_) => return Err(Box::new(context)),
    };
    let deadline = native_open_deadline(state);
    let dispatch = Arc::new(crate::workspace::Dispatch::default());
    let grant = owner_grant(
        state.events.clone(),
        context.trace.clone(),
        live.clone(),
        deadline,
        false,
    );
    let hide_grant = owner_grant(
        state.events.clone(),
        context.trace.clone(),
        live.clone(),
        deadline,
        true,
    );
    let hide_live = live.clone();
    let hide_dispatch = dispatch.clone();
    let revoked = context._services.live();
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
    context.capture_result = Some(saved);
    match state.execution.admit_open_prepared_tracked(
        Box::new(move || Ok(request)),
        mode,
        false,
        live,
        deadline,
        before,
        dispatch,
        context,
    ) {
        Ok(_) => Ok(()),
        Err((_, context)) => {
            ui.cancel_open_hide();
            Err(Box::new(context))
        }
    }
}
