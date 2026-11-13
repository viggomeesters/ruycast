//! Worker-owned action/editor state. Drafts bind to identity/base, never the
//! later palette selection. Only validated field edits reach the private writer.
use crate::{
    profile::{ConfigStore, SaveReceipt},
    session::Session,
};
use rustcast_core::Command;
use serde_json::{json, Value};
use std::path::PathBuf;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Mode {
    Alias,
    Hotkey,
    Remove,
    Edit,
    Input,
}
#[derive(Clone, Debug, PartialEq)]
pub struct Target {
    pub id: String,
    pub catalog_revision: u64,
    pub epoch: u64,
}
#[derive(Clone, Debug, PartialEq)]
pub struct Item {
    pub id: &'static str,
    pub title: String,
    pub shortcut: Option<&'static str>,
}
#[derive(Clone, Debug, PartialEq)]
pub struct EditorView {
    pub saving: bool,
    pub template_fields: Option<crate::template_form::Fields>,
    pub token: u64,
    pub id: String,
    pub title: String,
    pub mode: Mode,
    pub aliases: String,
    pub hotkey: Option<String>,
    pub error: String,
    pub attempt: u64,
    pub command: Value,
    pub is_new: bool,
    pub origin_menu: Option<u64>,
    pub origin_inline: Option<u64>,
    pub inline: bool,
    pub run_inline: bool,
    pub input_menu_attempt: Option<u64>,
    pub resumed_input_from: Option<u64>,
    pub suspended_input_token: Option<u64>,
    pub input_disabled: bool,
    pub task_date: Option<crate::task_date::TaskDate>,
}
#[derive(Clone, Debug)]
pub struct InlineSubmission {
    pub token: u64,
    pub attempt: u64,
    pub aliases: Option<String>,
    pub hotkey: Option<Option<String>>,
}
pub struct InlineOutcome {
    pub warning: Option<String>,
    pub error: Option<String>,
    pub opened: bool,
}
struct Draft {
    alive: std::sync::Arc<std::sync::atomic::AtomicBool>,
    discovered_app: bool,
    discovered_service: bool,
    view: EditorView,
    base: Command,
    auto_quit_base: Value,
    file: PathBuf,
    input_reference_now: f64,
}
impl Draft {
    fn date_preview(&self, value: &str, now: f64) -> Result<Option<crate::task_date::TaskDate>, String> {
        if self.view.mode != Mode::Input || !crate::task_date::supports(&self.base) {
            return Ok(None);
        }
        crate::execution::budget(&self.base, Some(value))?;
        if let Some(preview) = &self.view.task_date {
            if preview.input == value { return Ok(Some(preview.clone())); }
        }
        crate::task_date::resolve(value, if self.view.run_inline { now } else { self.input_reference_now }).map(Some)
    }
}
fn input_now() -> f64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs_f64())
        .unwrap_or_else(|error| -error.duration().as_secs_f64())
}
impl Clone for Draft {
    fn clone(&self) -> Self {
        Self {
            alive: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true)),
            discovered_app: self.discovered_app,
            discovered_service: self.discovered_service,
            view: self.view.clone(),
            base: self.base.clone(),
            auto_quit_base: self.auto_quit_base.clone(),
            file: self.file.clone(),
            input_reference_now: self.input_reference_now,
        }
    }
}
impl Drop for Draft {
    fn drop(&mut self) {
        self.alive
            .store(false, std::sync::atomic::Ordering::Release);
    }
}
#[derive(Default, Clone)]
pub struct Editing {
    editor: Option<Draft>,
    suspended_input: Option<Draft>,
    serial: std::sync::Arc<std::sync::atomic::AtomicU64>,
    pub action_selection: usize,
    pub menu_token: u64,
}
impl Editing {
    pub fn liveness_guard(&self) -> Option<crate::execution::Live> {
        self.editor.as_ref().map(|draft| {
            let alive = draft.alive.clone();
            std::sync::Arc::new(move || alive.load(std::sync::atomic::Ordering::Acquire))
                as crate::execution::Live
        })
    }
    pub fn set_saving(&mut self, saving: bool) {
        if let Some(draft) = self.editor.as_mut() {
            draft.view.saving = saving;
        }
    }
    pub fn is_saving(&self) -> bool {
        self.editor.as_ref().is_some_and(|d| d.view.saving)
    }
    fn next_token(&self) -> u64 {
        self.serial
            .fetch_update(
                std::sync::atomic::Ordering::AcqRel,
                std::sync::atomic::Ordering::Acquire,
                |n| n.checked_add(1),
            )
            .expect("Editor token space exhausted")
            + 1
    }

    pub fn begin_menu(&mut self) {
        let serial = self.next_token();
        self.menu_token = serial;
        self.action_selection = 0;
    }
    pub fn view(&self) -> Option<EditorView> {
        self.editor.as_ref().map(|d| d.view.clone())
    }
    pub fn token(&self) -> Option<u64> {
        self.editor.as_ref().map(|d| d.view.token)
    }
    pub fn cancel(&mut self, token: u64) {
        if self.token() == Some(token) {
            self.editor = None;
            self.resume_input();
        }
    }
    pub fn cancel_all(&mut self) {
        self.editor = None;
        self.suspended_input = None;
        self.action_selection = 0;
    }
    pub fn open(&mut self, store: &ConfigStore, command: &Command, mode: Mode) {
        self.open_at(store, command, mode, input_now()).expect("A finite owner clock can open a draft");
    }
    pub(crate) fn open_at(&mut self, store: &ConfigStore, command: &Command, mode: Mode, now: f64) -> Result<(), String> {
        let task_date = if mode == Mode::Input && crate::task_date::supports(command) {
            Some(crate::task_date::resolve("", now)?)
        } else { None };
        let serial = self.next_token();
        self.editor = Some(Draft {
            alive: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true)),
            discovered_app: command.kind == "app"
                && !store.current().commands.iter().any(|c| c.id == command.id),
            discovered_service: service_binding(command)
                && !store.current().commands.iter().any(|c|c.id==command.id),
            view: EditorView {
                saving: false,
                template_fields: None,
                token: serial,
                id: command.id.clone(),
                title: command.title.clone(),
                mode,
                aliases: command.aliases.join(", "),
                hotkey: command.hotkey.clone(),
                error: String::new(),
                attempt: 0,
                is_new: false,
                origin_menu: None,
                origin_inline: None,
                inline: false,
                run_inline: false,
                input_menu_attempt: None,
                resumed_input_from: None,
                suspended_input_token: None,
                input_disabled: mode == Mode::Input && command.enabled == Some(false),
                task_date,
                command: if mode == Mode::Input {
                    json!({"input":"", "inputPrompt":command.input_prompt.as_deref().filter(|p|!p.trim().is_empty()).unwrap_or("Input")})
                } else if mode == Mode::Edit {
                    let mut value = serde_json::to_value(command).expect("Command serialization");
                    if command.kind == "app" {
                        value["_autoQuit"] = auto_quit_preference(store, &command.target);
                    }
                    value
                } else {
                    Value::Null
                },
            },
            base: command.clone(),
            auto_quit_base: auto_quit_preference(store, &command.target),
            file: store.file.clone(),
            input_reference_now: now,
        });
        Ok(())
    }
    pub fn open_template(
        &mut self,
        store: &ConfigStore,
        command: &Command,
        fields: crate::template_form::Fields,
    ) {
        self.open(store, command, Mode::Input);
        let view = &mut self.editor.as_mut().expect("opened template").view;
        if fields.0.len() == 1 && fields.0[0].options.is_empty() {
            let field = &fields.0[0];
            view.command =
                json!({"input":field.default_value,"inputPrompt":field.label,"inputKey":field.id});
            view.run_inline = true;
        } else {
            view.template_fields = Some(fields);
        }
        let draft = self.editor.as_mut().expect("opened template");
        match draft.date_preview(draft.view.command["input"].as_str().unwrap_or(""), draft.input_reference_now) {
            Ok(preview) => draft.view.task_date = preview,
            Err(error) => { draft.view.task_date = None; draft.view.error = error; }
        }
    }
    /// Exactly one ephemeral run draft can underlie a configuration editor.
    pub fn edit_input(&mut self, store: &mut ConfigStore, mode: Mode) -> Result<(), String> {
        if self.suspended_input.is_some() || mode == Mode::Input {
            return Err("Finish the open editor first.".into());
        }
        let token = self.token().ok_or("This input is no longer open.")?;
        let (base, _, file) = self.input(token).ok_or("This input is no longer open.")?;
        store.reload_limited(Some(32_000_000))?;
        if file != store.file {
            return Err("Configuration folder changed. Reopen the command before editing.".into());
        }
        let current = store
            .current()
            .commands
            .iter()
            .find(|c| binding_identity(base, c))
            .ok_or("Command changed. Reopen it before editing.")?;
        crate::execution::budget(current, None)?;
        let current = current.clone();
        self.suspended_input = self.editor.take();
        self.open(store, &current, mode);
        self.editor
            .as_mut()
            .expect("child editor")
            .view
            .suspended_input_token = Some(token);
        Ok(())
    }
    pub fn enable_input(
        &mut self,
        store: &mut ConfigStore,
        enabled: bool,
    ) -> Result<SaveReceipt, String> {
        let token = self.token().ok_or("This input is no longer open.")?;
        let (base, _, file) = self.input(token).ok_or("This input is no longer open.")?;
        store.reload_limited(Some(32_000_000))?;
        if file != store.file
            || !store
                .current()
                .commands
                .iter()
                .any(|c| binding_identity(base, c))
        {
            return Err("Command changed. Reopen it before editing.".into());
        }
        let receipt = set_enabled(store, base, enabled)?;
        // Own receipt changes only enabled; never adopt other merged fields.
        self.editor.as_mut().expect("input").base.enabled = Some(enabled);
        self.editor.as_mut().expect("input").view.input_disabled = !enabled;
        Ok(receipt)
    }
    fn resume_input(&mut self) {
        if let Some(mut draft) = self.suspended_input.take() {
            let old = draft.view.token;
            let serial = self.next_token();
            draft.view.token = serial;
            draft.view.resumed_input_from = Some(old);
            draft.view.origin_inline = None;
            draft.view.origin_menu = None;
            draft.view.input_menu_attempt = None;
            draft.view.attempt = 0;
            self.editor = Some(draft);
        }
    }
    pub fn commit_inline(
        &mut self,
        store: &mut ConfigStore,
        previous: Option<InlineSubmission>,
    ) -> Result<Option<String>, String> {
        if let Some(draft) = self.editor.as_ref() {
            let previous = previous.ok_or("Finish the open editor first.")?;
            if !draft.view.inline || previous.token != draft.view.token {
                return Err("This editor is no longer open.".into());
            }
            self.update_draft(
                previous.token,
                previous.attempt,
                previous.aliases,
                previous.hotkey,
            );
            Ok(self.save(store, previous.token, None, None)?.warning)
        } else if previous.is_some() {
            Err("This editor is no longer open.".into())
        } else {
            Ok(None)
        }
    }
    pub fn transition_inline(
        &mut self,
        store: &mut ConfigStore,
        command: &Command,
        mode: Mode,
        previous: Option<InlineSubmission>,
        allow_followup: impl FnOnce() -> bool,
    ) -> Result<InlineOutcome, String> {
        if ![Mode::Alias, Mode::Hotkey, Mode::Edit].contains(&mode) {
            return Err("Unsupported inline field.".into());
        }
        let origin = previous.as_ref().map(|p| p.token);
        let warning = self.commit_inline(store, previous)?;
        if !allow_followup() {
            return Ok(InlineOutcome {
                warning,
                error: None,
                opened: false,
            });
        }
        let followup = (|| {
            let current = store
                .current()
                .commands
                .iter()
                .find(|c| c.id == command.id)
                .ok_or("Command was removed. Reopen it before editing.")?;
            if current.kind != command.kind
                || current.target != command.target
                || current.arguments != command.arguments
            {
                return Err("Command changed. Reopen it before editing.".to_string());
            }
            let current = current.clone();
            self.open(store, &current, mode);
            let editor = &mut self.editor.as_mut().expect("opened editor").view;
            editor.inline = mode != Mode::Edit;
            editor.origin_inline = origin;
            Ok(())
        })();
        Ok(InlineOutcome {
            warning,
            error: followup.as_ref().err().cloned(),
            opened: followup.is_ok(),
        })
    }
    pub fn cancelled(&self, ui: &crate::connected::Connection) -> bool {
        self.editor.as_ref().is_some_and(|d| {
            ui.editor_cancelled(d.view.token)
                || d.view.origin_inline.is_some_and(|t| ui.editor_cancelled(t))
                || d.view.origin_menu.is_some_and(|t| ui.menu_cancelled(t))
        })
    }
    pub fn acknowledge_inline(&mut self, previous: &InlineSubmission) -> bool {
        if self
            .editor
            .as_ref()
            .is_some_and(|d| d.view.inline && d.view.token == previous.token)
        {
            self.update_draft(
                previous.token,
                previous.attempt,
                previous.aliases.clone(),
                previous.hotkey.clone(),
            );
            true
        } else {
            false
        }
    }
    pub fn bind_origin_inline(&mut self, origin: Option<u64>) {
        if let Some(d) = self.editor.as_mut() {
            d.view.origin_inline = origin;
        }
    }
    pub fn origin_menu(&self) -> Option<u64> {
        self.editor.as_ref().and_then(|d| d.view.origin_menu)
    }
    pub fn bind_origin_menu(&mut self, token: Option<u64>) {
        if let Some(d) = &mut self.editor {
            d.view.origin_menu = token;
        }
    }
    pub fn create(
        &mut self,
        store: &ConfigStore,
        kind: &str,
        clipboard: Option<String>,
    ) -> Result<(), String> {
        if self.editor.is_some() {
            return Err("Finish the open editor first.".into());
        }
        if !["snippet", "url", "script"].contains(&kind) {
            return Err("Unsupported new command type.".into());
        }
        let id = objc2::rc::autoreleasepool(|_| {
            format!(
                "{}.{}",
                kind,
                objc2_foundation::NSUUID::new()
                    .UUIDString()
                    .to_string()
                    .to_lowercase()
            )
        });
        if store.current().commands.iter().any(|c| c.id == id) {
            return Err("Could not allocate a new command identity. Try again.".into());
        }
        let target = match kind {
            "snippet" => "".to_string(),
            "script" => "/usr/bin/true".to_string(),
            _ => clipboard
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| "https://".into()),
        };
        let title = match kind {
            "snippet" => "New Snippet",
            "script" => "New Shell Command",
            _ => "New Quicklink",
        };
        let command = Command {
            id,
            title: title.into(),
            kind: kind.into(),
            target,
            aliases: vec![],
            arguments: vec![],
            hotkey: None,
            timeout: None,
            input_prompt: None,
            keyword: None,
            application_id: None,
            enabled: None,
            snippet_output: None,
            imported_hotkey: None,
        };
        self.open(store, &command, Mode::Edit);
        self.editor.as_mut().unwrap().view.is_new = true;
        Ok(())
    }
    pub fn prefill_created(&mut self, token: u64, text: String) {
        if let Some(d) = self.editor.as_mut().filter(|d| {
            d.view.token == token
                && d.view.is_new
                && d.view.mode == Mode::Edit
                && d.base.kind == "url"
                && d.view.command["target"] == "https://"
        }) {
            d.view.command["target"] = json!(text);
        }
    }
    pub fn update_draft(
        &mut self,
        token: u64,
        attempt: u64,
        aliases: Option<String>,
        hotkey: Option<Option<String>>,
    ) {
        if let Some(d) = self.editor.as_mut().filter(|d| d.view.token == token) {
            d.view.attempt = attempt;
            if let Some(aliases) = aliases {
                d.view.aliases = aliases;
            }
            if let Some(hotkey) = hotkey {
                d.view.hotkey = hotkey;
            }
        }
    }
    pub fn update_command(&mut self, token: u64, command: Value) {
        if let Some(draft) = self
            .editor
            .as_mut()
            .filter(|d| d.view.token == token && d.view.mode == Mode::Edit)
        {
            draft.view.command = command;
        }
    }
    pub fn begin_input_menu(&mut self, attempt: u64) {
        self.begin_menu();
        if let Some(d) = self.editor.as_mut() {
            d.view.input_menu_attempt = Some(attempt);
        }
    }
    pub fn input_menu_attempt(&self) -> Option<u64> {
        self.editor.as_ref().and_then(|d| d.view.input_menu_attempt)
    }
    pub fn set_run_inline(&mut self, token: u64, inline: bool) {
        if let Some(draft) = self
            .editor
            .as_mut()
            .filter(|d| d.view.token == token && d.view.mode == Mode::Input)
        {
            draft.view.run_inline = inline;
        }
    }
    pub fn capture_input(&mut self, token:u64){
        if let Some(draft)=self.editor.as_mut().filter(|draft|draft.view.token==token&&draft.view.mode==Mode::Input&&(draft.base.target=="utility-set-text"||matches!(draft.base.id.as_str(),"capture.quick-note"|"capture.quick-task"))){
            draft.view.command["captureInput"]=json!(true);
        }
    }
    pub fn run_inline(&self) -> bool {
        self.editor
            .as_ref()
            .is_some_and(|d| d.view.mode == Mode::Input && d.view.run_inline)
    }
    pub fn update_input(&mut self, token: u64, attempt: u64, value: &str) -> Result<(), String> {
        self.update_input_at(token, attempt, value, input_now())
    }
    pub(crate) fn update_input_at(&mut self, token: u64, attempt: u64, value: &str, now: f64) -> Result<(), String> {
        if let Some(draft) = self
            .editor
            .as_mut()
            .filter(|d| d.view.token == token && d.view.mode == Mode::Input)
        {
            let preview = draft.date_preview(value, now)?;
            draft.view.attempt = attempt;
            draft.view.command["input"] = json!(value);
            draft.view.task_date = preview;
            draft.view.error.clear();
            return Ok(());
        }
        Err("This input is no longer open.".into())
    }
    /// A run draft never enters the configuration writer.
    pub fn input(&self, token: u64) -> Option<(&Command, &str, &std::path::Path)> {
        let draft = self
            .editor
            .as_ref()
            .filter(|d| d.view.token == token && d.view.mode == Mode::Input)?;
        Some((
            &draft.base,
            draft.view.command["input"].as_str()?,
            &draft.file,
        ))
    }
    /// Submission consumes the value already displayed; it never resolves a
    /// natural date again at Enter or falls back to an invalid literal date.
    pub fn input_execution_value(&self, token: u64) -> Result<String, String> {
        let (command, value, _) = self.input(token).ok_or("This input is no longer open.")?;
        if !crate::task_date::supports(command) { return Ok(value.to_owned()); }
        self.editor.as_ref().and_then(|draft| draft.view.task_date.as_ref())
            .filter(|preview| preview.input == value)
            .and_then(|preview| preview.value.clone())
            .ok_or_else(|| "Datum niet herkend".to_owned())
    }
    pub fn fail(&mut self, error: String) {
        if let Some(d) = &mut self.editor {
            d.view.error = error;
        }
    }
    pub fn save(
        &mut self,
        store: &mut ConfigStore,
        token: u64,
        aliases: Option<String>,
        hotkey: Option<Option<String>>,
    ) -> Result<SaveReceipt, String> {
        self.save_inner(store, token, aliases, hotkey, None)
    }
    pub fn save_with_apps(
        &mut self,
        store: &mut ConfigStore,
        token: u64,
        aliases: Option<String>,
        hotkey: Option<Option<String>>,
        apps: &crate::app_choice::Choices,
    ) -> Result<SaveReceipt, String> {
        self.save_inner(store, token, aliases, hotkey, Some(apps))
    }
    fn save_inner(
        &mut self,
        store: &mut ConfigStore,
        token: u64,
        aliases: Option<String>,
        hotkey: Option<Option<String>>,
        apps: Option<&crate::app_choice::Choices>,
    ) -> Result<SaveReceipt, String> {
        let draft = self
            .editor
            .as_mut()
            .filter(|d| d.view.token == token)
            .ok_or("This editor is no longer open.")?;
        if draft.view.mode == Mode::Input {
            return Err("Run input cannot be saved as configuration.".into());
        }
        if let Some(aliases) = aliases {
            draft.view.aliases = aliases;
        }
        if let Some(hotkey) = hotkey {
            draft.view.hotkey = hotkey;
        }
        let result = (|| {
            store.reload_limited(Some(32_000_000))?;
            if store.file != draft.file {
                return Err("Configuration folder changed. Reopen the item before editing.".into());
            }
            if draft.discovered_app && !draft.view.is_new {
                if draft.view.mode == Mode::Remove {
                    return Err("Discovered apps cannot be removed.".into());
                }
                validate_discovered_binding(store, &draft.base, apps)?;
            }
            if draft.discovered_service && draft.view.mode==Mode::Remove {
                return Err("Generated commands cannot be removed. Disable them instead.".into());
            }
            if draft.view.is_new {
                if draft.view.mode != Mode::Edit {
                    return Err("Invalid new command editor.".into());
                }
                if store
                    .current()
                    .commands
                    .iter()
                    .any(|c| c.id == draft.base.id)
                {
                    return Err("Command identity already exists. Reopen a new editor.".into());
                }
                let mut value: Value =
                    serde_json::from_slice(store.snapshot()).map_err(|e| e.to_string())?;
                let base = serde_json::to_value(&draft.base).map_err(|e| e.to_string())?;
                let mut edited = base.clone();
                merge_command(&mut edited, &base, &draft.view.command)?;
                value["commands"]
                    .as_array_mut()
                    .ok_or("Invalid command configuration")?
                    .push(edited);
                return store.save(&value);
            }
            let current = store
                .current()
                .commands
                .iter()
                .find(|c| c.id == draft.base.id)
                .cloned()
                .or_else(|| (draft.discovered_app || draft.discovered_service).then(|| draft.base.clone()))
                .ok_or("Command was removed. Reopen it before editing.")?;
            // A saved binding edits only its own field on the latest command.
            // Execution editors and unsaved discovery retain their pinned target.
            let saved_binding = matches!(draft.view.mode, Mode::Alias | Mode::Hotkey)
                && !draft.discovered_app
                && !draft.discovered_service;
            if current.kind != draft.base.kind
                || (!saved_binding
                    && (current.target != draft.base.target
                        || current.arguments != draft.base.arguments))
            {
                return Err("Command changed. Reopen it before editing.".into());
            }
            let mut value: Value =
                serde_json::from_slice(store.snapshot()).map_err(|e| e.to_string())?;
            let commands = value["commands"]
                .as_array_mut()
                .ok_or("Invalid command configuration.")?;
            let index = commands
                .iter()
                .position(|c| c["id"] == draft.base.id)
                .or_else(|| {
                    if draft.discovered_app || draft.discovered_service {
                        commands
                            .push(serde_json::to_value(&draft.base).expect("app serialization"));
                        Some(commands.len() - 1)
                    } else {
                        None
                    }
                })
                .ok_or("Command was removed. Reopen it before editing.")?;
            match draft.view.mode {
                Mode::Input => return Err("Run input cannot be saved as configuration.".into()),
                Mode::Alias => {
                    let edited: Vec<String> = if draft.view.aliases == draft.base.aliases.join(", ")
                    {
                        draft.base.aliases.clone()
                    } else {
                        draft
                            .view
                            .aliases
                            .split(',')
                            .map(str::trim)
                            .filter(|s| !s.is_empty())
                            .map(str::to_string)
                            .collect()
                    };
                    if current.aliases != draft.base.aliases && edited != current.aliases {
                        return Err(
                            "Aliases changed elsewhere. Reopen the item before editing.".into()
                        );
                    }
                    commands[index]["aliases"] = json!(edited);
                }
                Mode::Hotkey => {
                    if current.hotkey != draft.base.hotkey && draft.view.hotkey != current.hotkey {
                        return Err(
                            "Hotkey changed elsewhere. Reopen the item before editing.".into()
                        );
                    }
                    if let Some(key) = &draft.view.hotkey {
                        commands[index]["hotkey"] = json!(key);
                    } else {
                        commands[index]
                            .as_object_mut()
                            .ok_or("Invalid command")?
                            .remove("hotkey");
                    }
                }
                Mode::Edit => {
                    let base = serde_json::to_value(&draft.base).map_err(|e| e.to_string())?;
                    merge_command(&mut commands[index], &base, &draft.view.command)?;
                }
                Mode::Remove => {
                    let before = serde_json::to_value(&draft.base).map_err(|e| e.to_string())?;
                    let now = serde_json::to_value(&current).map_err(|e| e.to_string())?;
                    if before != now {
                        return Err("Command changed. Reopen it before removing.".into());
                    }
                    commands.remove(index);
                }
            }
            if draft.view.mode == Mode::Edit && draft.base.kind == "app" {
                let edited = &draft.view.command["_autoQuit"];
                if !edited.is_null() && edited != &draft.auto_quit_base {
                    if draft.view.command["target"] != draft.base.target {
                        return Err("Application changed. Save and reopen it before editing Auto Quit.".into());
                    }
                    let current = auto_quit_preference(store, &draft.base.target);
                    if current != draft.auto_quit_base && &current != edited {
                        return Err("Auto Quit changed elsewhere. Reopen the application before editing.".into());
                    }
                    if value.get("autoQuit").is_none() {
                        value["autoQuit"] = json!({});
                    }
                    let key = store.current().auto_quit.keys()
                        .find(|key| key.eq_ignore_ascii_case(&draft.base.target))
                        .cloned()
                        .unwrap_or_else(|| draft.base.target.clone());
                    value["autoQuit"][key] = edited.clone();
                }
            }
            store.save(&value)
        })();
        match result {
            Ok(receipt) => {
                if draft.view.mode == Mode::Remove {
                    self.suspended_input = None;
                } else if let Some(input) = self.suspended_input.as_mut() {
                    // A successful writer receipt owns only fields actually edited
                    // here. Concurrent external execution changes remain pinned to
                    // the old Input base and must fail the next Run revalidation.
                    let rebased = (|| {
                        let published = store
                            .current()
                            .commands
                            .iter()
                            .find(|c| c.id == draft.base.id)
                            .ok_or("Command saved. Reopen it before running.")?;
                        crate::execution::budget(published, None)?;
                        let mut value =
                            serde_json::to_value(published).map_err(|e| e.to_string())?;
                        let original =
                            serde_json::to_value(&input.base).map_err(|e| e.to_string())?;
                        let base = serde_json::to_value(&draft.base).map_err(|e| e.to_string())?;
                        let edited: Command = if draft.view.mode == Mode::Edit {
                            serde_json::from_value(command_document(&draft.view.command))
                                .map_err(|e| e.to_string())?
                        } else {
                            draft.base.clone()
                        };
                        let edited = serde_json::to_value(edited).map_err(|e| e.to_string())?;
                        for key in EXECUTION_FIELDS {
                            if base[key] == edited[key] {
                                if let Some(old) = original.get(key) {
                                    value[key] = old.clone();
                                } else {
                                    value.as_object_mut().expect("command object").remove(key);
                                }
                            }
                        }
                        let rebased: Command =
                            serde_json::from_value(value).map_err(|e| e.to_string())?;
                        crate::execution::budget(&rebased, None)?;
                        input.base = rebased;
                        input.view.task_date = input.date_preview(input.view.command["input"].as_str().unwrap_or(""), input.input_reference_now)?;
                        input.view.title = input.base.title.clone();
                        input.view.input_disabled = input.base.enabled == Some(false);
                        input.view.command["inputPrompt"] = json!(input
                            .base
                            .input_prompt
                            .as_deref()
                            .filter(|p| !p.trim().is_empty())
                            .unwrap_or("Input"));
                        Ok::<_, String>(())
                    })();
                    if let Err(error) = rebased {
                        input.view.error =
                            format!("Command saved. Reopen it before running. {error}");
                    }
                }
                self.editor = None;
                self.resume_input();
                Ok(receipt)
            }
            Err(error) => {
                draft.view.error = error.clone();
                Err(error)
            }
        }
    }
}
const EXECUTION_FIELDS: [&str; 8] = [
    "id",
    "kind",
    "target",
    "arguments",
    "timeout",
    "inputPrompt",
    "applicationID",
    "enabled",
];
pub fn binding_identity(a: &Command, b: &Command) -> bool {
    a.id == b.id
        && a.kind == b.kind
        && a.target == b.target
        && a.arguments == b.arguments
        && a.timeout == b.timeout
        && a.input_prompt == b.input_prompt
        && a.application_id == b.application_id
        && a.enabled == b.enabled
}
fn auto_quit_preference(store: &ConfigStore, application: &str) -> Value {
    store.current().auto_quit.iter()
        .find(|(key, _)| key.eq_ignore_ascii_case(application))
        .map(|(_, preference)| serde_json::to_value(preference).expect("preference serialization"))
        .unwrap_or_else(|| json!({"enabled":false,"minutes":30}))
}
/// `_autoQuit` travels only with the editor draft. It is never command data.
fn command_document(edited: &Value) -> Value {
    let mut document = edited.clone();
    if document["kind"] == "app" {
        if let Some(object) = document.as_object_mut() {
            object.remove("_autoQuit");
        }
    }
    document
}
/// Merge only fields changed in this editor. Optional-field presence in current
/// JSON is preserved when its typed value was not edited. IDs remain immutable.
fn merge_command(current: &mut Value, base: &Value, edited: &Value) -> Result<(), String> {
    let mut parsed: Command = serde_json::from_value(command_document(edited)).map_err(|e| e.to_string())?;
    if edited["id"] != base["id"] || edited["kind"] != base["kind"] {
        return Err("Command identity changed. Reopen it before editing.".into());
    }
    if ["url", "path"].contains(&parsed.kind.as_str()) {
        parsed.target = parsed.target.trim().to_string();
        if parsed.target.starts_with('/') || parsed.target.starts_with("~/") {
            parsed.kind = "path".into();
        } else if parsed.target.starts_with("file:") {
            parsed.target = objc2::rc::autoreleasepool(|_| {
                objc2_foundation::NSURL::URLWithString(&objc2_foundation::NSString::from_str(
                    &parsed.target,
                ))
                .filter(|u| u.isFileURL())
                .and_then(|u| u.path())
                .map(|p| p.to_string())
            })
            .ok_or("Invalid file URL")?;
            parsed.kind = "path".into();
        } else {
            parsed.kind = "url".into();
            if !parsed.target.contains(':') {
                parsed.target = format!("https://{}", parsed.target);
            }
        }
    }
    let canonical = serde_json::to_value(parsed).map_err(|e| e.to_string())?;
    let fields = base
        .as_object()
        .ok_or("Invalid command")?
        .keys()
        .chain(canonical.as_object().ok_or("Invalid command")?.keys())
        .cloned()
        .collect::<std::collections::BTreeSet<_>>();
    for field in fields {
        if canonical.get(&field) == base.get(&field) {
            continue;
        }
        // Compare typed defaults to prevent absent aliases/arguments from looking
        // externally edited merely because serialization materializes defaults.
        let typed: Command = serde_json::from_value(current.clone()).map_err(|e| e.to_string())?;
        let typed = serde_json::to_value(typed).map_err(|e| e.to_string())?;
        if typed.get(&field) != base.get(&field) && typed.get(&field) != canonical.get(&field) {
            return Err(format!(
                "{} changed elsewhere. Reopen the item before editing.",
                field
            ));
        }
        if let Some(value) = canonical.get(&field) {
            current[&field] = value.clone();
        } else {
            current
                .as_object_mut()
                .ok_or("Invalid command")?
                .remove(&field);
        }
    }
    Ok(())
}
pub fn target(session: &Session, epoch: u64) -> Option<Target> {
    session.selected().map(|c| Target {
        id: c.id.clone(),
        catalog_revision: session.results_revision(),
        epoch,
    })
}
pub fn matches(target: &Target, session: &Session, epoch: u64) -> bool {
    target.epoch == epoch
        && target.catalog_revision == session.results_revision()
        && session.selected().is_some_and(|c| c.id == target.id)
}
pub fn global(name: &str) -> bool {
    ["settings", "clipboard.settings", "new-snippet", "new-quicklink", "new-script", "refresh-system", "safari-access"].contains(&name)
}
pub fn items(command: Option<&Command>, saved: bool) -> Vec<Item> {
    let mut items = vec![];
    if let Some(c) = command {
        if c.enabled != Some(false) {
            items.push(Item {
                id: "open",
                title: crate::presentation::action_title(c).into(),
                shortcut: None,
            });
        }
        if ["snippet", "url", "path"].contains(&c.kind.as_str()) {
            items.push(Item {
                id: "copy-template",
                title: if c.kind == "snippet" {
                    "Copy Template"
                } else {
                    "Copy Link"
                }
                .into(),
                shortcut: None,
            });
        }
        if !c.id.starts_with("system.") {
            for (id, title, shortcut) in [
                ("edit", "Edit", None),
                ("set-alias", "Set Alias…", Some("cmd+alt+,")),
                ("set-hotkey", "Set Hotkey…", Some("cmd+shift+,")),
                if c.enabled == Some(false) {
                    ("enable", "Enable", None)
                } else {
                    ("disable", "Disable", None)
                },
            ] {
                items.push(Item {
                    id,
                    title: title.into(),
                    shortcut,
                });
            }
            if saved {
                items.push(Item {
                    id: "remove",
                    title: "Remove".into(),
                    shortcut: None,
                });
            }
        }
    }
    items.push(Item {
        id: "settings",
        title: "Settings".into(),
        shortcut: None,
    });
    for (id, title) in [
        ("new-snippet", "New Snippet…"),
        ("new-quicklink", "New Quicklink…"),
        ("new-script", "New Shell Command…"),
    ] {
        items.push(Item {
            id,
            title: title.into(),
            shortcut: None,
        });
    }
    items
}
pub fn disable(store: &mut ConfigStore, command: &Command) -> Result<SaveReceipt, String> {
    set_enabled(store, command, false)
}
fn validate_discovered_binding(
    store: &ConfigStore,
    command: &Command,
    apps: Option<&crate::app_choice::Choices>,
) -> Result<(), String> {
    if !apps.is_some_and(|apps| apps.contains_origin(command)) {
        return Err("App discovery changed. Reopen the item before editing.".into());
    }
    if !store.current().commands.iter().any(|c| c.id == command.id)
        && !crate::applications::available(&store.current().commands, command)
    {
        return Err("App settings changed elsewhere. Reopen the item before editing.".into());
    }
    Ok(())
}
pub fn set_enabled(
    store: &mut ConfigStore,
    command: &Command,
    enabled: bool,
) -> Result<SaveReceipt, String> {
    set_enabled_inner(store, command, enabled, None)
}
pub fn set_enabled_with_apps(
    store: &mut ConfigStore,
    command: &Command,
    enabled: bool,
    apps: &crate::app_choice::Choices,
) -> Result<SaveReceipt, String> {
    set_enabled_inner(store, command, enabled, Some(apps))
}
fn set_enabled_inner(
    store: &mut ConfigStore,
    command: &Command,
    enabled: bool,
    apps: Option<&crate::app_choice::Choices>,
) -> Result<SaveReceipt, String> {
    // Fix origin before reload: a saved app deleted elsewhere must stay deleted.
    let discovered =
        command.kind == "app" && !store.current().commands.iter().any(|c| c.id == command.id);
    let discovered_shortcut = service_binding(command) && !store.current().commands.iter().any(|c|c.id==command.id);
    let original_file = store.file.clone();
    store.reload_limited(Some(32_000_000))?;
    if store.file != original_file {
        return Err("Configuration folder changed. Reopen the command before editing.".into());
    }
    if discovered {
        validate_discovered_binding(store, command, apps)?;
    }
    let current = store
        .current()
        .commands
        .iter()
        .find(|c| c.id == command.id)
        .or_else(|| (discovered || discovered_shortcut).then_some(command))
        .ok_or("Command was removed. Reopen it before editing.")?;
    if current.target != command.target
        || current.kind != command.kind
        || current.arguments != command.arguments
    {
        return Err("Command changed. Reopen it before editing.".into());
    }
    if (current.enabled != Some(false)) != (command.enabled != Some(false)) {
        return Err("Enabled state changed elsewhere. Try again.".into());
    }
    let mut value: Value = serde_json::from_slice(store.snapshot()).map_err(|e| e.to_string())?;
    let rows = value["commands"]
        .as_array_mut()
        .ok_or("Invalid command configuration.")?;
    let index = if let Some(index) = rows.iter().position(|c| c["id"] == command.id) {
        index
    } else if discovered || discovered_shortcut {
        rows.push(serde_json::to_value(command).map_err(|e| e.to_string())?);
        rows.len() - 1
    } else {
        return Err("Command was removed. Reopen it before editing.".into());
    };
    rows[index]["enabled"] = json!(enabled);
    store.save(&value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::profile::ProfilePaths;
    use std::{
        fs,
        sync::atomic::{AtomicUsize, Ordering},
    };
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    struct Temp(PathBuf);
    impl Temp {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!(
                "rc-actions-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir(&path).unwrap();
            Self(path)
        }
        fn store(&self) -> ConfigStore {
            ConfigStore::open(ProfilePaths::new(self.0.join("profile"), self.0.clone(), None))
            .unwrap()
        }
    }
    impl Drop for Temp {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.0).unwrap();
        }
    }
    #[test]
    fn auto_quit_editor_merges_only_its_application_and_refuses_conflicts() {
        let temp = Temp::new();
        let mut store = temp.store();
        let command = store.current().commands[0].clone();
        assert_eq!(command.kind, "app");
        let mut editing = Editing::default();
        editing.open(&store, &command, Mode::Edit);
        let token = editing.token().unwrap();
        let mut edited = editing.view().unwrap().command;
        edited["_autoQuit"] = json!({"enabled":true,"minutes":17});
        edited["title"] = json!("Own application");
        editing.update_command(token, edited);
        let mut external: Value = serde_json::from_slice(store.snapshot()).unwrap();
        external["clipboard"]["limit"] = json!(87);
        external["autoQuit"] = json!({"test.synthetic.Other":{"enabled":true,"minutes":44}});
        store.save(&external).unwrap();
        editing.save(&mut store, token, None, None).unwrap();
        assert_eq!(store.current().auto_quit[&command.target].minutes, 17);
        assert_eq!(store.current().auto_quit["test.synthetic.Other"].minutes, 44);
        assert_eq!(store.current().clipboard.limit, 87);
        assert_eq!(store.current().commands[0].title, "Own application");
        assert!(!String::from_utf8_lossy(store.snapshot()).contains("_autoQuit"));
        let command = store.current().commands[0].clone();
        editing.open(&store, &command, Mode::Edit);
        let token = editing.token().unwrap();
        let mut edited = editing.view().unwrap().command;
        edited["_autoQuit"]["minutes"] = json!(20);
        editing.update_command(token, edited);
        let mut external: Value = serde_json::from_slice(store.snapshot()).unwrap();
        external["autoQuit"][&command.target]["minutes"] = json!(25);
        store.save(&external).unwrap();
        let saved = store.snapshot().to_vec();
        assert!(editing.save(&mut store, token, None, None).unwrap_err().contains("Auto Quit changed elsewhere"));
        assert_eq!(store.snapshot(), saved);
        assert_eq!(editing.view().unwrap().command["_autoQuit"]["minutes"], 20);
        editing.cancel(token);
        editing.open(&store, &command, Mode::Edit);
        let token = editing.token().unwrap();
        let mut edited = editing.view().unwrap().command;
        edited["target"] = json!("test.synthetic.Replacement");
        edited["_autoQuit"]["minutes"] = json!(30);
        editing.update_command(token, edited);
        assert!(editing.save(&mut store, token, None, None).unwrap_err().contains("Application changed"));
        assert_eq!(store.snapshot(), saved);
    }
    #[test]
    fn suspended_input_uses_fresh_tokens_and_rejects_stale_child_callbacks() {
        let temp = Temp::new();
        let mut store = temp.store();
        let command = store.current().commands[0].clone();
        let mut editing = Editing::default();
        editing.open(&store, &command, Mode::Input);
        let initial = editing.token().unwrap();
        editing.set_run_inline(initial, true);
        editing.update_input(initial, 1, "PRIVATE Ω").unwrap();
        editing.edit_input(&mut store, Mode::Alias).unwrap();
        let child = editing.token().unwrap();
        assert!(editing.edit_input(&mut store, Mode::Hotkey).is_err());
        editing.bind_origin_menu(Some(9));
        editing.cancel(child);
        let resumed = editing.token().unwrap();
        assert!(resumed > child && child > initial);
        let view = editing.view().unwrap();
        assert_eq!(view.resumed_input_from, Some(initial));
        assert_eq!(view.origin_menu, None);
        assert_eq!(view.input_menu_attempt, None);
        editing.cancel(child);
        assert!(editing
            .save(&mut store, child, Some("stale".into()), None)
            .is_err());
        assert_eq!(editing.input(resumed).unwrap().1, "PRIVATE Ω");
        editing.edit_input(&mut store, Mode::Alias).unwrap();
        editing.cancel_all();
        assert!(editing.token().is_none());
        assert!(editing.suspended_input.is_none());
    }
    #[test]
    fn own_edit_rebases_only_edited_execution_fields_after_publication() {
        let temp = Temp::new();
        let mut store = temp.store();
        let command = store.current().commands[0].clone();
        let mut editing = Editing::default();
        editing.open(&store, &command, Mode::Input);
        editing.edit_input(&mut store, Mode::Edit).unwrap();
        let child = editing.token().unwrap();
        let mut edited = serde_json::to_value(&command).unwrap();
        edited["title"] = json!("Own title");
        edited["timeout"] = json!(42);
        editing.update_command(child, edited);
        let mut external: Value = serde_json::from_slice(store.snapshot()).unwrap();
        external["commands"][0]["inputPrompt"] = json!("External prompt");
        store.save(&external).unwrap();
        editing.save(&mut store, child, None, None).unwrap();
        let resumed = editing.token().unwrap();
        let base = editing.input(resumed).unwrap().0;
        assert_eq!(base.timeout, Some(42.));
        assert_eq!(base.title, "Own title");
        assert_eq!(base.input_prompt, command.input_prompt);
        assert_eq!(
            store.current().commands[0].input_prompt.as_deref(),
            Some("External prompt")
        );
        assert!(!binding_identity(base, &store.current().commands[0]));
    }
    #[test]
    fn capture_input_classification_preserves_original_ids_targets_and_token_mode_boundaries() {
        let temp = Temp::new();
        let store = temp.store();
        for (id, kind, target, capture) in [
            ("capture.quick-note", "script", "/owned/override", true),
            ("capture.quick-task", "script", "/owned/other", true),
            ("capture.quick-note", "builtin", "owned-override", true),
            ("ordinary", "builtin", "utility-set-text", true),
            ("capture.quick-note.extra", "script", "/owned/override", false),
            ("ordinary", "script", "/owned/quick-note", false),
        ] {
            let mut base = store.current().commands[0].clone();
            base.id = id.into();
            base.kind = kind.into();
            base.target = target.into();
            base.input_prompt = Some("Owned input".into());
            let mut edit = Editing::default();
            edit.open(&store, &base, Mode::Input);
            let token = edit.token().unwrap();
            edit.capture_input(token + 1);
            assert_ne!(edit.view().unwrap().command["captureInput"], true);
            edit.capture_input(token);
            assert_eq!(edit.view().unwrap().command["captureInput"] == true, capture);
            edit.open(&store, &base, Mode::Edit);
            edit.capture_input(edit.token().unwrap());
            assert_ne!(edit.view().unwrap().command["captureInput"], true);
        }
    }
    #[test]
    fn task_date_submission_keeps_displayed_clock_on_same_value_and_clone() {
        let temp = Temp::new();
        let store = temp.store();
        let mut base = store.current().commands[0].clone();
        base.kind = "script".into();
        base.target = "/bin/echo".into();
        base.input_prompt = Some("Deadline YYYY-MM-DD".into());
        let mut edit = Editing::default();
        edit.open_at(&store, &base, Mode::Input, 946_728_000.0).unwrap();
        let token = edit.token().unwrap();
        edit.update_input_at(token, 1, "morgen", 1_800_000_000.0).unwrap();
        let displayed = edit.view().unwrap().task_date.unwrap();
        assert_eq!(displayed.value.as_deref(), Some("2000-01-02"));
        assert_eq!(edit.input(token).unwrap().1, "morgen");
        edit.set_run_inline(token, true);
        edit.update_input_at(token, 2, "morgen", 1_900_000_000.0).unwrap();
        assert_eq!(edit.view().unwrap().task_date, Some(displayed.clone()));
        assert_eq!(edit.input_execution_value(token).unwrap(), "2000-01-02");
        let cloned = edit.clone();
        assert_eq!(cloned.input_execution_value(token).unwrap(), "2000-01-02");
        edit.update_input_at(token, 3, "today", 1_800_000_000.0).unwrap();
        assert_eq!(edit.view().unwrap().task_date, Some(crate::task_date::resolve("today", 1_800_000_000.0).unwrap()));
        assert_eq!(cloned.view().unwrap().task_date, Some(displayed));
        let before = edit.view().unwrap();
        assert!(edit.update_input_at(token, 4, "tomorrow", f64::NAN).is_err());
        assert_eq!(edit.view(), Some(before));
        edit.update_input_at(token, 5, "in 3", 1_800_000_000.0).unwrap();
        assert_eq!(edit.input_execution_value(token).unwrap_err(), "Datum niet herkend");
        assert!(!edit.view().unwrap().task_date.unwrap().is_valid());
        edit.update_input_at(token, 6, "clear", 1_800_000_000.0).unwrap();
        assert_eq!(edit.input_execution_value(token).unwrap(), "clear");
        edit.cancel(token);
        assert!(edit.input_execution_value(token).is_err());
    }
    #[test]
    fn task_date_opt_in_does_not_change_generic_literal_input_or_command_json() {
        let temp = Temp::new();
        let store = temp.store();
        let mut base = store.current().commands[0].clone();
        base.kind = "script".into();
        base.input_prompt = Some("Message".into());
        let mut edit = Editing::default();
        edit.open_at(&store, &base, Mode::Input, 946_728_000.0).unwrap();
        let token = edit.token().unwrap();
        assert_eq!(edit.view().unwrap().command, json!({"input":"","inputPrompt":"Message"}));
        edit.update_input_at(token, 1, "morgen Ω\nraw", 1_800_000_000.0).unwrap();
        assert!(edit.view().unwrap().task_date.is_none());
        assert_eq!(edit.input_execution_value(token).unwrap(), "morgen Ω\nraw");
        assert_eq!(edit.view().unwrap().command, json!({"input":"morgen Ω\nraw","inputPrompt":"Message"}));
        base.input_prompt = Some("YYYY-MM-DD\u{0301}".into());
        edit.open_at(&store, &base, Mode::Input, 946_728_000.0).unwrap();
        let token = edit.token().unwrap();
        edit.update_input_at(token, 1, "morgen", 1_800_000_000.0).unwrap();
        assert!(edit.view().unwrap().task_date.is_none());
        assert_eq!(edit.input_execution_value(token).unwrap(), "morgen");
    }
    #[test]
    fn task_date_suspend_resume_and_own_rebase_keep_or_remove_the_real_preview() {
        let temp = Temp::new();
        let mut store = temp.store();
        let mut raw: Value = serde_json::from_slice(store.snapshot()).unwrap();
        raw["commands"][0]["kind"] = json!("script");
        raw["commands"][0]["target"] = json!("/bin/echo");
        raw["commands"][0]["inputPrompt"] = json!("Deadline YYYY-MM-DD");
        store.save(&raw).unwrap();
        let base = store.current().commands[0].clone();
        let mut edit = Editing::default();
        edit.open_at(&store, &base, Mode::Input, 946_728_000.0).unwrap();
        let token = edit.token().unwrap();
        edit.set_run_inline(token, true);
        edit.update_input_at(token, 1, "morgen", 946_728_000.0).unwrap();
        let preview = edit.view().unwrap().task_date.clone();
        edit.edit_input(&mut store, Mode::Alias).unwrap();
        let child = edit.token().unwrap();
        edit.cancel(child);
        let resumed = edit.token().unwrap();
        assert_ne!(resumed, token);
        assert_eq!(edit.view().unwrap().task_date, preview);
        edit.update_input_at(resumed, 1, "morgen", 1_800_000_000.0).unwrap();
        assert_eq!(edit.input_execution_value(resumed).unwrap(), "2000-01-02");
        assert!(edit.update_input_at(token, 2, "clear", 1_800_000_000.0).is_err());
        edit.edit_input(&mut store, Mode::Edit).unwrap();
        let child = edit.token().unwrap();
        let mut command = serde_json::to_value(&base).unwrap();
        command["inputPrompt"] = json!("Message");
        edit.update_command(child, command);
        edit.save(&mut store, child, None, None).unwrap();
        let resumed = edit.token().unwrap();
        assert!(edit.view().unwrap().task_date.is_none());
        assert_eq!(edit.input_execution_value(resumed).unwrap(), "morgen");
    }
    #[test]
    fn input_draft_is_bounded_ephemeral_and_never_enters_writer() {
        let temp = Temp::new();
        let mut store = temp.store();
        let mut base = store.current().commands[0].clone();
        base.kind = "script".into();
        base.target = "/usr/bin/true".into();
        base.input_prompt = Some("Message".into());
        let bytes = fs::read(&store.file).unwrap();
        let mut edit = Editing::default();
        edit.open(&store, &base, Mode::Input);
        let token = edit.token().unwrap();
        assert_eq!(
            edit.view().unwrap().command,
            json!({"input":"", "inputPrompt":"Message"})
        );
        edit.update_input(token, 1, "literal Ω\nline").unwrap();
        assert_eq!(edit.input(token).unwrap().1, "literal Ω\nline");
        assert!(edit
            .update_input(token, 2, &"x".repeat(1024 * 1024 + 1))
            .is_ok());
        assert_eq!(edit.view().unwrap().attempt, 2);
        assert_eq!(edit.input(token).unwrap().1.len(), 1024 * 1024 + 1);
        assert!(edit.save(&mut store, token, None, None).is_err());
        assert_eq!(fs::read(&store.file).unwrap(), bytes);
        edit.cancel(token);
        edit.open(&store, &base, Mode::Input);
        assert!(edit.update_input(token, 3, "stale").is_err());
        assert_eq!(edit.input(edit.token().unwrap()).unwrap().1, "");
    }
    fn discovered() -> Command {
        serde_json::from_value(json!({"id":"discovered.fixture","title":"Fixture App","kind":"app","target":"dev.fixture.app","aliases":[]})).unwrap()
    }
    #[test]
    fn discovered_alias_and_hotkey_persist_without_losing_external_fields() {
        let temp = Temp::new();
        let mut store = temp.store();
        let base = discovered();
        let apps = crate::app_choice::Choices::new(std::slice::from_ref(&base));
        let mut edit = Editing::default();
        edit.open(&store, &base, Mode::Alias);
        let token = edit.token().unwrap();
        external(&store, |v| {
            v["theme"] = json!("light");
        });
        edit.save_with_apps(&mut store, token, Some("fixturealias".into()), None, &apps)
            .unwrap();
        let saved = store
            .current()
            .commands
            .iter()
            .find(|c| c.id == base.id)
            .unwrap()
            .clone();
        assert_eq!(saved.aliases, vec!["fixturealias"]);
        edit.open(&store, &saved, Mode::Hotkey);
        let token = edit.token().unwrap();
        edit.save_with_apps(
            &mut store,
            token,
            None,
            Some(Some("cmd+shift+9".into())),
            &apps,
        )
        .unwrap();
        let value: Value = serde_json::from_slice(store.snapshot()).unwrap();
        assert_eq!(value["theme"], "light");
        assert_eq!(
            store
                .current()
                .commands
                .iter()
                .find(|c| c.id == base.id)
                .unwrap()
                .hotkey
                .as_deref(),
            Some("cmd+shift+9")
        );
    }
    #[test]
    fn discovered_bindings_require_current_identity_and_refuse_external_override() {
        let temp = Temp::new();
        let mut store = temp.store();
        let base = discovered();
        let mut edit = Editing::default();
        let apps = crate::app_choice::Choices::new(std::slice::from_ref(&base));
        edit.open(&store, &base, Mode::Alias);
        let token = edit.token().unwrap();
        let before = fs::read(&store.file).unwrap();
        assert!(edit
            .save(&mut store, token, Some("mine".into()), None)
            .is_err());
        assert_eq!(fs::read(&store.file).unwrap(), before);
        let mut other = base.clone();
        other.id = "different.identity".into();
        assert!(edit
            .save_with_apps(
                &mut store,
                token,
                None,
                None,
                &crate::app_choice::Choices::new(&[other.clone()])
            )
            .is_err());
        external(&store, |v| {
            v["commands"]
                .as_array_mut()
                .unwrap()
                .push(serde_json::to_value(other).unwrap())
        });
        let before = fs::read(&store.file).unwrap();
        assert!(edit
            .save_with_apps(&mut store, token, None, None, &apps)
            .unwrap_err()
            .contains("elsewhere"));
        assert_eq!(fs::read(&store.file).unwrap(), before);
    }
    #[test]
    fn discovered_disable_is_saved_and_saved_app_removal_is_not_resurrected() {
        let temp = Temp::new();
        let mut store = temp.store();
        let base = discovered();
        let apps = crate::app_choice::Choices::new(std::slice::from_ref(&base));
        set_enabled_with_apps(&mut store, &base, false, &apps).unwrap();
        let saved = store
            .current()
            .commands
            .iter()
            .find(|c| c.id == base.id)
            .unwrap()
            .clone();
        assert_eq!(saved.enabled, Some(false));
        let mut edit = Editing::default();
        edit.open(&store, &saved, Mode::Alias);
        let token = edit.token().unwrap();
        external(&store, |v| {
            v["commands"]
                .as_array_mut()
                .unwrap()
                .retain(|c| c["id"] != base.id)
        });
        let before = fs::read(&store.file).unwrap();
        assert!(edit
            .save_with_apps(&mut store, token, Some("mine".into()), None, &apps)
            .is_err());
        assert_eq!(fs::read(&store.file).unwrap(), before);
    }
    #[test]
    fn enabling_merges_unrelated_external_edits() {
        let temp = Temp::new();
        let mut store = temp.store();
        let base = store.current().commands[0].clone();
        external(&store, |v| {
            v["commands"][0]["title"] = json!("Outside title");
            v["clipboard"]["limit"] = json!(81);
        });
        set_enabled(&mut store, &base, false).unwrap();
        assert_eq!(store.current().commands[0].title, "Outside title");
        assert_eq!(store.current().commands[0].enabled, Some(false));
        assert_eq!(
            serde_json::from_slice::<Value>(store.snapshot()).unwrap()["clipboard"]["limit"],
            81
        );
    }
    #[test]
    fn enabling_refuses_an_externally_changed_enabled_field() {
        let temp = Temp::new();
        let mut store = temp.store();
        let base = store.current().commands[0].clone();
        external(&store, |v| v["commands"][0]["enabled"] = json!(false));
        let before = std::fs::read(&store.file).unwrap();
        assert!(set_enabled(&mut store, &base, true)
            .unwrap_err()
            .contains("changed elsewhere"));
        assert_eq!(std::fs::read(&store.file).unwrap(), before);
    }
    #[test]
    fn enabling_refuses_a_relocated_configuration() {
        let temp = Temp::new();
        let mut store = temp.store();
        let base = store.current().commands[0].clone();
        let mut external = temp.store();
        let target = temp.0.join("moved");
        std::fs::create_dir(&target).unwrap();
        external.relocate(target.to_str().unwrap()).unwrap();
        let before = std::fs::read(&external.file).unwrap();
        assert!(set_enabled(&mut store, &base, false)
            .unwrap_err()
            .contains("folder changed"));
        assert_eq!(std::fs::read(&external.file).unwrap(), before);
    }
    fn external(store: &ConfigStore, change: impl FnOnce(&mut Value)) -> Value {
        let mut value: Value = serde_json::from_slice(store.snapshot()).unwrap();
        change(&mut value);
        fs::write(&store.file, serde_json::to_vec(&value).unwrap()).unwrap();
        value
    }
    #[test]
    fn inline_transition_commits_before_opening_the_next_bound_item() {
        let temp = Temp::new();
        let mut store = temp.store();
        let a = store.current().commands[0].clone();
        let b = store.current().commands[1].clone();
        let mut e = Editing::default();
        assert!(
            e.transition_inline(&mut store, &a, Mode::Alias, None, || true)
                .unwrap()
                .opened
        );
        let token = e.token().unwrap();
        let outcome = e
            .transition_inline(
                &mut store,
                &b,
                Mode::Hotkey,
                Some(InlineSubmission {
                    token,
                    attempt: 7,
                    aliases: Some("newfinder".into()),
                    hotkey: None,
                }),
                || true,
            )
            .unwrap();
        assert!(outcome.opened);
        assert!(outcome.error.is_none());
        assert_eq!(store.current().commands[0].aliases, vec!["newfinder"]);
        let draft = e.view().unwrap();
        assert_eq!(draft.id, b.id);
        assert_ne!(draft.token, token);
        assert!(draft.inline);
        assert_eq!(draft.origin_inline, Some(token));
        assert!(draft.command.is_null());
    }
    #[test]
    fn inline_invalid_alias_retains_the_first_draft_and_attempt() {
        let temp = Temp::new();
        let mut store = temp.store();
        let a = store.current().commands[0].clone();
        let b = store.current().commands[1].clone();
        let mut e = Editing::default();
        e.transition_inline(&mut store, &a, Mode::Alias, None, || true)
            .unwrap();
        let token = e.token().unwrap();
        let before = fs::read(&store.file).unwrap();
        assert!(e
            .transition_inline(
                &mut store,
                &b,
                Mode::Alias,
                Some(InlineSubmission {
                    token,
                    attempt: 9,
                    aliases: Some(b.aliases[0].clone()),
                    hotkey: None
                }),
                || true
            )
            .is_err());
        let d = e.view().unwrap();
        assert_eq!(d.token, token);
        assert_eq!(d.attempt, 9);
        assert_eq!(d.aliases, b.aliases[0]);
        assert!(!d.error.is_empty());
        assert_eq!(fs::read(&store.file).unwrap(), before);
    }
    #[test]
    fn inline_cancel_after_commit_prevents_followup_without_undoing_publication() {
        let temp = Temp::new();
        let mut store = temp.store();
        let a = store.current().commands[0].clone();
        let b = store.current().commands[1].clone();
        let mut e = Editing::default();
        e.transition_inline(&mut store, &a, Mode::Alias, None, || true)
            .unwrap();
        let token = e.token().unwrap();
        let file = store.file.clone();
        let result = e
            .transition_inline(
                &mut store,
                &b,
                Mode::Alias,
                Some(InlineSubmission {
                    token,
                    attempt: 1,
                    aliases: Some("already-published".into()),
                    hotkey: None,
                }),
                || {
                    assert_eq!(
                        serde_json::from_slice::<Value>(&fs::read(file).unwrap()).unwrap()
                            ["commands"][0]["aliases"],
                        json!(["already-published"])
                    );
                    false
                },
            )
            .unwrap();
        assert!(!result.opened);
        assert!(result.error.is_none());
        assert!(e.token().is_none());
    }
    #[test]
    fn inline_removed_next_item_preserves_committed_first_item_and_reports_error() {
        let temp = Temp::new();
        let mut store = temp.store();
        let a = store.current().commands[0].clone();
        let b = store.current().commands[1].clone();
        let mut e = Editing::default();
        e.transition_inline(&mut store, &a, Mode::Alias, None, || true)
            .unwrap();
        let token = e.token().unwrap();
        external(&store, |v| {
            v["commands"]
                .as_array_mut()
                .unwrap()
                .retain(|c| c["id"] != b.id)
        });
        let result = e
            .transition_inline(
                &mut store,
                &b,
                Mode::Alias,
                Some(InlineSubmission {
                    token,
                    attempt: 1,
                    aliases: Some("first-published".into()),
                    hotkey: None,
                }),
                || true,
            )
            .unwrap();
        assert!(!result.opened);
        assert!(result.error.unwrap().contains("removed"));
        assert!(e.token().is_none());
        assert_eq!(store.current().commands[0].aliases, vec!["first-published"]);
    }
    #[test]
    fn new_draft_never_publishes_before_valid_explicit_save() {
        let temp = Temp::new();
        let mut store = temp.store();
        let before = fs::read(&store.file).unwrap();
        let count = store.current().commands.len();
        let mut edit = Editing::default();
        edit.create(&store, "snippet", None).unwrap();
        let draft = edit.view().unwrap();
        assert!(draft.is_new);
        assert!(draft.id.starts_with("snippet."));
        assert_eq!(fs::read(&store.file).unwrap(), before);
        assert!(edit.save(&mut store, draft.token, None, None).is_err());
        assert_eq!(edit.view().unwrap().id, draft.id);
        assert_eq!(fs::read(&store.file).unwrap(), before);
        let mut value = draft.command;
        value["target"] = json!("Hello\nworld");
        edit.update_command(draft.token, value);
        edit.save(&mut store, draft.token, None, None).unwrap();
        assert_eq!(store.current().commands.len(), count + 1);
        assert!(edit.save(&mut store, draft.token, None, None).is_err());
        assert_eq!(store.current().commands.len(), count + 1);
    }
    #[test]
    fn new_identity_collision_never_replaces_an_existing_command() {
        let temp = Temp::new();
        let mut store = temp.store();
        let mut edit = Editing::default();
        edit.create(&store, "script", None).unwrap();
        let draft = edit.view().unwrap();
        let expected = external(&store, |v| {
            let mut row = draft.command.clone();
            row["title"] = json!("External owner");
            v["commands"].as_array_mut().unwrap().push(row);
        });
        assert!(edit
            .save(&mut store, draft.token, None, None)
            .unwrap_err()
            .contains("identity already exists"));
        assert!(edit.view().is_some());
        assert_eq!(
            serde_json::from_slice::<Value>(&fs::read(&store.file).unwrap()).unwrap(),
            expected
        );
    }
    #[test]
    fn new_command_preserves_unrelated_edits_and_rejects_invalid_kind_or_identity() {
        let temp = Temp::new();
        let mut store = temp.store();
        let mut edit = Editing::default();
        assert!(edit.create(&store, "app", None).is_err());
        edit.create(&store, "url", Some("  example.com  ".into()))
            .unwrap();
        let draft = edit.view().unwrap();
        assert_eq!(draft.command["target"], "example.com");
        assert!(edit.create(&store, "script", None).is_err());
        let mut changed = draft.command.clone();
        changed["id"] = json!("wrong");
        edit.update_command(draft.token, changed);
        assert!(edit.save(&mut store, draft.token, None, None).is_err());
        edit.update_command(draft.token, draft.command);
        external(&store, |v| v["clipboard"]["limit"] = json!(82));
        edit.save(&mut store, draft.token, None, None).unwrap();
        assert_eq!(store.current().clipboard.limit, 82);
        assert_eq!(
            store.current().commands.last().unwrap().target,
            "https://example.com"
        );
    }
    #[test]
    fn field_merge_preserves_other_command_and_global_changes() {
        let temp = Temp::new();
        let mut store = temp.store();
        let base = store.current().commands[0].clone();
        let mut edit = Editing::default();
        edit.open(&store, &base, Mode::Alias);
        let token = edit.token().unwrap();
        external(&store, |v| {
            v["clipboard"]["limit"] = json!(51);
            v["commands"][0]["title"] = json!("Renamed externally");
            v["commands"][1]["title"] = json!("Other external title");
        });
        edit.save(&mut store, token, Some("work, büro".into()), None)
            .unwrap();
        assert_eq!(store.current().clipboard.limit, 51);
        assert_eq!(store.current().commands[0].title, "Renamed externally");
        assert_eq!(store.current().commands[1].title, "Other external title");
        assert_eq!(store.current().commands[0].aliases, ["work", "büro"]);
        assert!(edit.view().is_none());
    }
    #[test]
    fn binding_save_preserves_external_execution_definition() {
        for mode in [Mode::Alias, Mode::Hotkey] {
            let temp = Temp::new();
            let mut store = temp.store();
            let base = store.current().commands[0].clone();
            let mut edit = Editing::default();
            edit.open(&store, &base, mode);
            let token = edit.token().unwrap();
            external(&store, |v| {
                v["commands"][0]["title"] = json!("External definition");
                v["commands"][0]["target"] = json!("com.example.external");
                v["commands"][0]["arguments"] = json!(["external argument"]);
                v["commands"][1]["title"] = json!("Other external title");
                v["clipboard"]["limit"] = json!(51);
            });
            let aliases = (mode == Mode::Alias).then(|| "feedback-binding".into());
            let hotkey = (mode == Mode::Hotkey).then(|| Some("ctrl+alt+f8".into()));
            edit.save(&mut store, token, aliases, hotkey).unwrap();
            let saved = &store.current().commands[0];
            assert_eq!(saved.title, "External definition");
            assert_eq!(saved.target, "com.example.external");
            assert_eq!(saved.arguments, ["external argument"]);
            if mode == Mode::Alias {
                assert_eq!(saved.aliases, ["feedback-binding"]);
                assert_eq!(saved.hotkey, base.hotkey);
            } else {
                assert_eq!(saved.hotkey.as_deref(), Some("ctrl+alt+f8"));
                assert_eq!(saved.aliases, base.aliases);
            }
            assert_eq!(store.current().commands[1].title, "Other external title");
            assert_eq!(store.current().clipboard.limit, 51);
            assert!(edit.view().is_none());
        }
    }
    #[test]
    fn same_field_external_conflict_retains_external_bytes_and_draft() {
        let temp = Temp::new();
        let mut store = temp.store();
        let base = store.current().commands[0].clone();
        let mut edit = Editing::default();
        edit.open(&store, &base, Mode::Alias);
        let token = edit.token().unwrap();
        external(&store, |v| v["commands"][0]["aliases"] = json!(["outside"]));
        let before = fs::read(&store.file).unwrap();
        assert!(edit
            .save(&mut store, token, Some("my draft".into()), None)
            .unwrap_err()
            .contains("changed elsewhere"));
        assert_eq!(fs::read(&store.file).unwrap(), before);
        assert_eq!(edit.view().unwrap().aliases, "my draft");
    }
    #[test]
    fn deleted_or_execution_editor_replaced_target_is_never_substituted() {
        for (mode, remove) in [
            (Mode::Alias, true), (Mode::Hotkey, true),
            (Mode::Edit, true), (Mode::Remove, true),
            (Mode::Edit, false), (Mode::Remove, false),
        ] {
            let temp = Temp::new();
            let mut store = temp.store();
            let base = store.current().commands[0].clone();
            let mut edit = Editing::default();
            edit.open(&store, &base, mode);
            let token = edit.token().unwrap();
            external(&store, |v| {
                if remove {
                    v["commands"].as_array_mut().unwrap().remove(0);
                } else {
                    v["commands"][0]["target"] = json!("com.example.replacement");
                }
            });
            let before = fs::read(&store.file).unwrap();
            assert!(edit
                .save(&mut store, token, Some("mine".into()), None)
                .is_err());
            assert_eq!(fs::read(&store.file).unwrap(), before);
            assert!(edit.view().is_some());
        }
    }
    #[test]
    fn binding_save_refuses_a_changed_command_kind() {
        for mode in [Mode::Alias, Mode::Hotkey] {
            let temp = Temp::new();
            let mut store = temp.store();
            let base = store.current().commands[0].clone();
            let mut edit = Editing::default();
            edit.open(&store, &base, mode);
            let token = edit.token().unwrap();
            external(&store, |v| {
                v["commands"][0]["kind"] = json!("url");
                v["commands"][0]["target"] = json!("https://example.invalid");
            });
            let before = fs::read(&store.file).unwrap();
            assert!(edit.save(&mut store, token, None, None).is_err());
            assert_eq!(fs::read(&store.file).unwrap(), before);
            assert!(edit.view().is_some());
        }
    }
    #[test]
    fn strict_validation_conflicts_do_not_publish_and_hotkey_draft_survives() {
        let temp = Temp::new();
        let mut store = temp.store();
        let base = store.current().commands[0].clone();
        let mut edit = Editing::default();
        edit.open(&store, &base, Mode::Alias);
        let token = edit.token().unwrap();
        let before = fs::read(&store.file).unwrap();
        assert!(edit
            .save(&mut store, token, Some("SAFARI".into()), None)
            .is_err());
        assert_eq!(fs::read(&store.file).unwrap(), before);
        edit.open(&store, &base, Mode::Hotkey);
        let token = edit.token().unwrap();
        let key = store.current().clipboard_hotkey.clone();
        assert!(edit
            .save(&mut store, token, None, Some(Some(key.clone())))
            .is_err());
        assert_eq!(edit.view().unwrap().hotkey.as_deref(), Some(key.as_str()));
        assert_eq!(fs::read(&store.file).unwrap(), before);
        external(&store, |v| {
            v["commands"][0]["unknownField"] = json!("private fixture")
        });
        let before = fs::read(&store.file).unwrap();
        assert!(edit.save(&mut store, token, None, Some(None)).is_err());
        assert_eq!(fs::read(&store.file).unwrap(), before);
    }
    #[test]
    fn portable_mapping_change_cannot_redirect_an_open_editor() {
        let temp = Temp::new();
        let mut store = temp.store();
        let base = store.current().commands[0].clone();
        let original = store.file.clone();
        let mut edit = Editing::default();
        edit.open(&store, &base, Mode::Alias);
        let token = edit.token().unwrap();
        let folder = temp.0.join("portable");
        fs::create_dir(&folder).unwrap();
        fs::write(folder.join("config.json"), store.snapshot()).unwrap();
        fs::write(
            store.paths.root.join("config-location.json"),
            serde_json::to_vec(&json!({"directory":folder})).unwrap(),
        )
        .unwrap();
        let before = fs::read(folder.join("config.json")).unwrap();
        assert!(edit
            .save(&mut store, token, Some("mine".into()), None)
            .unwrap_err()
            .contains("folder changed"));
        assert_eq!(fs::read(folder.join("config.json")).unwrap(), before);
        assert_eq!(fs::read(original).unwrap(), before);
    }
    #[test]
    fn removal_requires_unchanged_confirmed_command_and_current_token() {
        let temp = Temp::new();
        let mut store = temp.store();
        let base = store.current().commands[0].clone();
        let mut edit = Editing::default();
        edit.open(&store, &base, Mode::Remove);
        let token = edit.token().unwrap();
        external(&store, |v| v["commands"][0]["title"] = json!("Changed"));
        let before = fs::read(&store.file).unwrap();
        assert!(edit.save(&mut store, token, None, None).is_err());
        assert_eq!(fs::read(&store.file).unwrap(), before);
        edit.cancel(token);
        assert!(edit.save(&mut store, token, None, None).is_err());
        let current = store.current().commands[0].clone();
        edit.open(&store, &current, Mode::Remove);
        let token = edit.token().unwrap();
        edit.save(&mut store, token, None, None).unwrap();
        assert!(!store.current().commands.iter().any(|c| c.id == base.id));
    }
}

#[cfg(test)]
mod command_edit_tests {
    use super::*;
    fn command() -> Value {
        json!({"id":"test","title":"Title","kind":"script","target":"/usr/bin/printf","arguments":["literal\nargument","$(no expansion)","büro"],"aliases":[],"timeout":30.0})
    }
    #[test]
    fn merges_only_changed_fields_and_refuses_identity_and_same_field_conflicts() {
        let base = command();
        let mut current = base.clone();
        current["aliases"] = json!(["external"]);
        let mut edited = base.clone();
        edited["title"] = json!("Changed");
        merge_command(&mut current, &base, &edited).unwrap();
        assert_eq!(current["aliases"], json!(["external"]));
        assert_eq!(current["arguments"], base["arguments"]);
        assert_eq!(current["title"], "Changed");
        let mut current = base.clone();
        current["title"] = json!("Other");
        assert!(merge_command(&mut current, &base, &edited)
            .unwrap_err()
            .contains("changed elsewhere"));
        edited["id"] = json!("replacement");
        assert!(merge_command(&mut current, &base, &edited).is_err());
    }
    #[test]
    fn rejects_unknown_fields_and_preserves_absent_defaults() {
        let mut raw = command();
        raw.as_object_mut().unwrap().remove("arguments");
        raw.as_object_mut().unwrap().remove("aliases");
        let typed: Command = serde_json::from_value(raw.clone()).unwrap();
        let base = serde_json::to_value(typed).unwrap();
        let mut edited = base.clone();
        edited["title"] = json!("New");
        merge_command(&mut raw, &base, &edited).unwrap();
        assert!(raw.get("aliases").is_none());
        assert!(raw.get("arguments").is_none());
        edited["unknown"] = json!(true);
        assert!(merge_command(&mut raw, &base, &edited).is_err());
    }
    #[test]
    fn quicklink_normalizes_domain_path_and_file_url() {
        for (target, kind, expected) in [
            (" example.com ", "url", "https://example.com"),
            ("~/büro", "path", "~/büro"),
            ("file:///tmp/hello%20world", "path", "/tmp/hello world"),
        ] {
            let base = json!({"id":"test","title":"Link","kind":"url","target":"https://example.com/","aliases":[],"arguments":[]});
            let mut edited = base.clone();
            edited["target"] = json!(target);
            let mut current = base.clone();
            merge_command(&mut current, &base, &edited).unwrap();
            assert_eq!(current["kind"], kind);
            assert_eq!(current["target"], expected);
        }
    }
}

/// Unsaved service bindings have a fixed source execution tuple. Capture this
/// origin before reload so deleting an already-saved row never resurrects it.
fn service_binding(c:&Command)->bool {
    crate::system_search::apple_shortcuts::recognizes(c) || (c.kind=="builtin" && c.arguments.is_empty() && [
        ("builtin.settings","settings"),("builtin.dashboard","analytics"),("builtin.open-config","open-config"),("builtin.reload-config","reload-config"),("builtin.system-monitor","system-monitor"),
        ("builtin.window-search","switch-windows"),("builtin.menu-actions","search-menu-items"),("builtin.safari-tabs","search-safari-tabs"),("builtin.apple-shortcuts","apple-shortcuts"),
    ].iter().any(|&(id,target)|c.id==id && c.target==target))
}
