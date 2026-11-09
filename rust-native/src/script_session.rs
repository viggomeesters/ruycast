//! Serialized rich-result ownership. The main thread gets labels and detail,
//! never executable bindings or choice values. Admission and completion are
//! tied to the original session, including when a provider reuses choice IDs.
use crate::script_result::{ChoiceSession, Interpreted};
use rustcast_core::Command;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq, serde::Serialize)]
pub struct Label {
    pub id: String,
    pub title: String,
    pub subtitle: String,
}
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
pub struct Surface {
    pub token: u64,
    pub title: String,
    pub placeholder: String,
    pub query: Option<String>,
    pub query_valid: bool,
    pub rows: Vec<Label>,
    pub selection: Option<usize>,
    pub detail: Option<String>,
    pub busy: bool,
    pub detail_visible: bool,
    pub has_choices: bool,
    pub error: String,
}
/// No Serialize/Debug: origins contain private executable arguments.
#[derive(Clone)]
pub struct Origin {
    token: u64,
    ticket: u64,
    command: Command,
    file: PathBuf,
}
impl Origin {
    pub fn expected_envelope(&self) -> bool {
        true
    }
    pub fn token(&self) -> u64 {
        self.token
    }
    pub fn command(&self) -> &Command {
        &self.command
    }
    pub fn file(&self) -> &Path {
        &self.file
    }
}
/// Private owner-to-worker request; a reload always starts with empty input.
pub struct Request {
    origin: Origin,
    revision: u64,
    prepared: Command,
    values: Value,
}
struct Session {
    token: u64,
    command: Command,
    file: PathBuf,
    choices: Option<ChoiceSession>,
    detail: Option<String>,
    query: Option<String>,
    query_error: Option<String>,
    error: String,
    revision: u64,
    pending: Option<u64>,
    detail_visible: bool,
}
#[derive(Default)]
pub struct Owner {
    serial: u64,
    active: Option<Session>,
}
#[derive(Clone, Debug, PartialEq)]
pub enum Completion {
    Stale,
    Kept,
    Replaced,
    Closed { notice: String },
}
fn next(serial: &mut u64) -> u64 {
    // Never reuse an identity. Exhaustion cannot authorize old callbacks.
    *serial = serial
        .checked_add(1)
        .expect("script result identity exhausted");
    *serial
}
impl Owner {
    pub fn fail(&mut self, token: u64, error: &str) {
        if let Some(s) = self.active.as_mut().filter(|s| s.token == token) {
            s.error = error.into();
            s.revision = s
                .revision
                .checked_add(1)
                .expect("script error revision exhausted");
        }
    }
    pub fn stamp(&self) -> (u64, bool, u64, Option<u64>) {
        (
            self.serial,
            self.active.is_some(),
            self.active.as_ref().map_or(0, |s| s.revision),
            self.active.as_ref().and_then(|s| s.pending),
        )
    }
    pub fn token(&self) -> Option<u64> {
        self.active.as_ref().map(|s| s.token)
    }
    pub fn selected_id(&self, token: u64) -> Option<&str> {
        self.active
            .as_ref()
            .filter(|s| s.token == token && s.query_error.is_none() && !s.detail_visible)
            .and_then(|s| s.choices.as_ref())
            .and_then(|c| c.selected())
            .map(|c| c.id.as_str())
    }
    pub fn command_id(&self, token: u64) -> Option<&str> {
        self.active
            .as_ref()
            .filter(|s| s.token == token)
            .map(|s| s.command.id.as_str())
    }
    pub fn show_detail(&mut self, token: u64, show: bool) -> bool {
        let Ok(s) = self.editable(token) else {
            return false;
        };
        if (show && s.detail.is_none()) || (!show && s.choices.is_none()) {
            return false;
        }
        s.detail_visible = show;
        s.revision = s
            .revision
            .checked_add(1)
            .expect("script detail revision exhausted");
        true
    }
    pub fn surface(&self) -> Option<Surface> {
        let s = self.active.as_ref()?;
        Some(Surface {
            token: s.token,
            title: s
                .choices
                .as_ref()
                .map(|c| {
                    if c.choices().title.trim().is_empty() {
                        s.command.title.clone()
                    } else {
                        c.choices().title.clone()
                    }
                })
                .unwrap_or_else(|| s.command.title.clone()),
            placeholder: s
                .choices
                .as_ref()
                .map(|c| c.choices().placeholder.clone())
                .unwrap_or_default(),
            query: s.query.clone(),
            query_valid: s.query_error.is_none(),
            rows: s
                .choices
                .as_ref()
                .map(|c| {
                    c.rows()
                        .map(|r| Label {
                            id: r.id.clone(),
                            title: r.title.clone(),
                            subtitle: r.subtitle.clone(),
                        })
                        .collect()
                })
                .unwrap_or_default(),
            // Rejected text remains repairable, but a hidden older selection is
            // not represented as runnable under the rejected query.
            selection: s
                .query_error
                .is_none()
                .then(|| s.choices.as_ref().and_then(ChoiceSession::selection))
                .flatten(),
            detail: s.detail.clone(),
            busy: s.pending.is_some(),
            detail_visible: s.detail_visible,
            has_choices: s.choices.is_some(),
            error: s.query_error.clone().unwrap_or_else(|| s.error.clone()),
        })
    }
    /// Caller first proves initial job/window/epoch/dismissal/intent ownership.
    /// A failed initial result may present detail while its Input is retained by
    /// the window owner. This reducer never owns or discards ordinary Input.
    pub fn present(
        &mut self,
        command: &Command,
        file: &Path,
        result: Interpreted,
        warning: Option<&str>,
    ) -> Result<bool, String> {
        crate::execution::eligible(command)?;
        crate::execution::budget(command, None)?;
        if !file.is_absolute() {
            return Err("Invalid script result owner.".into());
        }
        if result.choices.is_none() && result.detail.is_none() {
            return Ok(false);
        }
        let token = next(&mut self.serial);
        let choices = result
            .choices
            .map(|c| ChoiceSession::new(token, command, file.to_owned(), c))
            .transpose()?;
        let detail_visible = choices.is_none();
        self.active = Some(Session {
            token,
            command: command.clone(),
            file: file.to_owned(),
            choices,
            detail: result.detail,
            query: Some(String::new()),
            query_error: None,
            error: result_error(&result.response, warning),
            revision: 0,
            detail_visible,
            pending: None,
        });
        Ok(true)
    }
    pub fn back(&mut self, token: u64) -> bool {
        if self.active.as_ref().is_some_and(|s| s.token == token) {
            self.active = None;
            true
        } else {
            false
        }
    }
    pub fn clear(&mut self) {
        self.active = None;
    }
    fn editable(&mut self, token: u64) -> Result<&mut Session, String> {
        let s = self
            .active
            .as_mut()
            .filter(|s| s.token == token)
            .ok_or("This result is no longer open.")?;
        if s.pending.is_some() {
            return Err("A command is already running in this result.".into());
        }
        Ok(s)
    }
    pub fn query(&mut self, token: u64, query: &str) -> Result<(), String> {
        let s = self.editable(token)?;
        if s.detail_visible {
            return Err("This result is read-only.".into());
        }
        let c = s.choices.as_mut().ok_or("This result has no choices.")?;
        // Store bounded repairable text. Oversize queries are not cloned into
        // the owner; the native field retains the submitted text for repair.
        s.revision = s
            .revision
            .checked_add(1)
            .expect("script query revision exhausted");
        s.query = (query.len() <= 65_536).then(|| query.into());
        match c.set_query(query) {
            Ok(()) => {
                s.query_error = None;
                Ok(())
            }
            Err(error) => {
                s.query_error = Some(error.clone());
                Err(error)
            }
        }
    }
    pub fn select(&mut self, token: u64, id: &str) -> bool {
        let Ok(s) = self.editable(token) else {
            return false;
        };
        if s.query_error.is_some() || s.detail_visible {
            return false;
        }
        let selected = s.choices.as_mut().is_some_and(|c| c.select(id));
        if selected {
            s.revision = s
                .revision
                .checked_add(1)
                .expect("script selection revision exhausted");
        }
        selected
    }
    pub fn move_selection(&mut self, token: u64, delta: isize) -> bool {
        let Ok(s) = self.editable(token) else {
            return false;
        };
        if s.query_error.is_some() || s.detail_visible {
            return false;
        }
        let Some(c) = s.choices.as_mut() else {
            return false;
        };
        c.move_selection(delta);
        s.revision = s
            .revision
            .checked_add(1)
            .expect("script selection revision exhausted");
        true
    }
    /// `id=None` means reload, never the last selected value. Callers must
    /// reread the actual mapped configuration before invoking this method.
    pub fn prepare(
        &mut self,
        token: u64,
        id: Option<&str>,
        current: &Command,
        file: &Path,
    ) -> Result<Request, String> {
        let s = self.editable(token)?;
        if s.file != file
            || !crate::actions::binding_identity(&s.command, current)
            || current.enabled == Some(false)
        {
            return Err("Command changed. Reopen it before running.".into());
        }
        if s.choices.is_none() || s.detail_visible {
            return Err("This result has no choices.".into());
        }
        let input = if let Some(id) = id {
            if let Some(error) = &s.query_error {
                return Err(error.clone());
            }
            Some(
                s.choices
                    .as_ref()
                    .and_then(|c| c.value(token, id))
                    .ok_or("Selection changed. Try again.")?,
            )
        } else {
            None
        };
        let prepared = crate::execution::prepare_result(current, input)?;
        let values = input
            .map(|i| json!({"input":i}))
            .unwrap_or_else(|| json!({}));
        let command = s.command.clone();
        let file = s.file.clone();
        let revision = s.revision;
        let ticket = next(&mut self.serial);
        Ok(Request {
            origin: Origin {
                token,
                ticket,
                command,
                file,
            },
            revision,
            prepared,
            values,
        })
    }
    /// Run on the serialized owner with the freshly mapped current command/file.
    /// Do not carry a prepared request across a queued/yielded turn without
    /// rereading configuration. Both preparation and admission validate it.
    /// A failed worker admission leaves all
    /// presentation state untouched. The closure gets the exact pinned origin
    /// to place in its execution context, before any worker can complete.
    pub fn admit(
        &mut self,
        request: Request,
        current: &Command,
        file: &Path,
        admit: impl FnOnce(&Command, &Value, &Origin) -> Result<(), String>,
    ) -> Result<Origin, String> {
        let s = self.editable(request.origin.token)?;
        if request.origin.file != file
            || !crate::actions::binding_identity(&request.origin.command, current)
            || current.enabled == Some(false)
        {
            return Err("Command changed. Reopen it before running.".into());
        }
        if s.revision != request.revision {
            return Err("Selection changed. Try again.".into());
        }
        admit(&request.prepared, &request.values, &request.origin)?;
        s.pending = Some(request.origin.ticket);
        // Retire every other prepared intent from before this admission. Even
        // if this job later fails and keeps the same selection, a queued old
        // request cannot start another invocation without a fresh user intent.
        s.revision = s
            .revision
            .checked_add(1)
            .expect("script admission revision exhausted");
        s.error.clear();
        Ok(request.origin)
    }
    /// `result` is interpreted using the originating job's required-envelope
    /// bit. A later usage-store warning is separate from provider success, so
    /// valid new presentation is never lost after the effect completed.
    pub fn complete(
        &mut self,
        origin: &Origin,
        mut result: Interpreted,
        warning: Option<&str>,
    ) -> Result<Completion, String> {
        let Some(s) = self
            .active
            .as_mut()
            .filter(|s| s.token == origin.token && s.pending == Some(origin.ticket))
        else {
            return Ok(Completion::Stale);
        };
        s.pending = None;
        s.revision = s
            .revision
            .checked_add(1)
            .expect("script completion revision exhausted");
        if result.response["ok"] != true && result.response["actionCompleted"] == true {
            // The effect may already have happened. Never put its old choices
            // back in a runnable state. Detail may survive in a fresh readonly
            // result; otherwise return the concrete notice to the window.
            result.choices = None;
            if result.detail.is_some() {
                self.present(&origin.command, &origin.file, result, warning)?;
                return Ok(Completion::Replaced);
            }
            let notice = result_error(&result.response, warning);
            self.active = None;
            return Ok(Completion::Closed { notice });
        }
        if result.response["ok"] != true {
            s.error = result_error(&result.response, warning);
            if result.detail.is_some() {
                s.detail = result.detail;
            }
            return Ok(Completion::Kept);
        }
        if result.choices.is_none() && result.detail.is_none() {
            self.active = None;
            return Ok(Completion::Closed {
                notice: result_error(&result.response, warning),
            });
        }
        self.present(&origin.command, &origin.file, result, warning)?;
        Ok(Completion::Replaced)
    }
}
fn result_error(response: &Value, warning: Option<&str>) -> String {
    let error =
        (response["ok"] != true).then(|| response["error"].as_str().unwrap_or("Script failed."));
    error
        .into_iter()
        .chain(warning.filter(|w| !w.is_empty()))
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::script_result::{interpret, Audience};
    fn command() -> Command {
        serde_json::from_value(json!({"id":"fixture","title":"Fixture","kind":"script",
            "target":"/usr/bin/printf","arguments":["{input}","prefix{input}","{uuid}"],
            "inputPrompt":"Message"}))
        .unwrap()
    }
    fn result(value: Value) -> Interpreted {
        interpret(
            json!({"ok":true,"result":{"stdout":value.to_string(),"exitCode":0}}),
            true,
            Audience::Native,
            Some(&rustcast_core::script::Capture::empty()),
        )
    }
    fn choices(value: &str) -> Interpreted {
        result(
            json!({"ok":true,"choices":{"title":"Projects","placeholder":"Find project",
            "items":[{"id":"reused","title":"Équipe","subtitle":"Road Map","value":value},
                {"id":"two","title":"Second","subtitle":"Other","value":"SECOND"}]},"detail":"Initial detail"}),
        )
    }
    fn owner() -> (Owner, Command, PathBuf, u64) {
        let mut o = Owner::default();
        let c = command();
        let f = PathBuf::from("/own/config.json");
        assert!(o.present(&c, &f, choices("$(literal)\nΩ"), None).unwrap());
        let t = o.surface().unwrap().token;
        (o, c, f, t)
    }
    fn pending(o: &mut Owner, c: &Command, f: &Path, t: u64) -> Origin {
        let r = o.prepare(t, Some("reused"), c, f).unwrap();
        o.admit(r, c, f, |_, _, _| Ok(())).unwrap()
    }
    #[test]
    fn closing_or_clearing_fresh_result_always_changes_publication_stamp() {
        let (mut o, c, f, t) = owner();
        let before = o.stamp();
        assert!(o.back(t));
        assert_ne!(before, o.stamp());
        o.present(&c, &f, choices("fresh"), None).unwrap();
        let before = o.stamp();
        o.clear();
        assert_ne!(before, o.stamp());
    }
    #[test]
    fn surface_has_only_metadata_and_independent_detail() {
        let (o, _, _, t) = owner();
        let s = o.surface().unwrap();
        assert_eq!(s.token, t);
        assert_eq!(s.rows.len(), 2);
        assert_eq!(s.rows[0].title, "Équipe");
        assert_eq!(s.selection, Some(0));
        assert_eq!(s.detail.as_deref(), Some("Initial detail"));
        // Debug output is limited to the plain surface. No command, argument,
        // literal choice value, or mapped file was copied there.
        let surface = format!("{s:?}");
        for private in [
            "$(literal)",
            "prefix{input}",
            "/usr/bin/printf",
            "/own/config.json",
        ] {
            assert!(!surface.contains(private));
        }
    }
    #[test]
    fn selected_value_is_literal_and_empty_choice_value_is_allowed() {
        let (mut o, c, f, t) = owner();
        let r = o.prepare(t, Some("reused"), &c, &f).unwrap();
        let origin = o
            .admit(r, &c, &f, |p, v, origin| {
                assert_eq!(p.arguments[0], "$(literal)\nΩ");
                assert_eq!(p.arguments[1], "prefix{input}");
                assert_eq!(*v, json!({"input":"$(literal)\nΩ"}));
                assert!(origin.expected_envelope());
                assert_eq!(origin.command().target, "/usr/bin/printf");
                assert_eq!(origin.file(), f);
                Ok(())
            })
            .unwrap();
        o.complete(&origin, choices(""), None).unwrap();
        let t = o.surface().unwrap().token;
        let r = o.prepare(t, Some("reused"), &c, &f).unwrap();
        o.admit(r, &c, &f, |p, v, _| {
            assert_eq!(p.arguments[0], "");
            assert_eq!(*v, json!({"input":""}));
            Ok(())
        })
        .unwrap();
    }
    #[test]
    fn reload_uses_empty_input_and_fresh_uuid_never_last_selected_value() {
        let (mut o, c, f, t) = owner();
        assert!(o.select(t, "two"));
        let r = o.prepare(t, None, &c, &f).unwrap();
        let first = r.prepared.arguments[2].clone();
        o.admit(r, &c, &f, |p, v, _| {
            assert_eq!(p.arguments[0], "");
            assert_eq!(*v, json!({}));
            Ok(())
        })
        .unwrap();
        assert!(o.prepare(t, None, &c, &f).is_err());
        o.clear();
        o.present(&c, &f, choices("another"), None).unwrap();
        let r = o.prepare(o.surface().unwrap().token, None, &c, &f).unwrap();
        assert_ne!(first, r.prepared.arguments[2]);
        assert!(crate::execution::prepare(&c, &json!({})).is_err());
    }
    #[test]
    fn actual_file_and_execution_identity_are_rechecked() {
        let (mut o, c, f, t) = owner();
        assert!(o
            .prepare(t, None, &c, Path::new("/other/config.json"))
            .is_err());
        for change in [
            "target",
            "arguments",
            "timeout",
            "inputPrompt",
            "enabled",
            "applicationID",
            "kind",
            "id",
        ] {
            let mut v = serde_json::to_value(&c).unwrap();
            v[change] = match change {
                "arguments" => json!(["changed"]),
                "timeout" => json!(1.0),
                "enabled" => json!(false),
                _ => json!("changed"),
            };
            let changed = serde_json::from_value(v).unwrap();
            assert!(o.prepare(t, None, &changed, &f).is_err(), "{change}");
        }
        let mut cosmetic = c.clone();
        cosmetic.title = "Renamed".into();
        cosmetic.aliases = vec!["newalias".into()];
        assert!(o.prepare(t, None, &cosmetic, &f).is_ok());
    }
    #[test]
    fn stale_query_selection_and_reused_id_cannot_admit_old_request() {
        let (mut o, c, f, t) = owner();
        let request = o.prepare(t, Some("reused"), &c, &f).unwrap();
        o.query(t, "other").unwrap();
        assert!(o
            .admit(request, &c, &f, |_, _, _| panic!("stale admission"))
            .is_err());
        assert!(o.prepare(t, Some("reused"), &c, &f).is_err());
        assert!(!o.select(t, "unknown"));
        o.present(&c, &f, choices("replacement"), None).unwrap();
        let new = o.surface().unwrap().token;
        assert_ne!(new, t);
        assert!(!o.back(t));
        assert!(o.query(t, "old").is_err());
        assert!(!o.select(t, "reused"));
        assert!(o.prepare(t, Some("reused"), &c, &f).is_err());
    }
    #[test]
    fn rejected_query_blocks_old_selection_and_is_repairable() {
        let (mut o, c, f, t) = owner();
        o.query(t, "EQUIPE road").unwrap();
        assert_eq!(o.surface().unwrap().rows.len(), 1);
        let invalid = (0..65)
            .map(|i| format!("word{i}"))
            .collect::<Vec<_>>()
            .join(" ");
        assert!(o.query(t, &invalid).is_err());
        let s = o.surface().unwrap();
        assert_eq!(s.query.as_deref(), Some(invalid.as_str()));
        assert!(!s.query_valid);
        assert_eq!(s.selection, None);
        assert!(o.prepare(t, Some("reused"), &c, &f).is_err());
        assert!(!o.select(t, "reused"));
        assert!(!o.move_selection(t, 1));
        assert!(o.prepare(t, None, &c, &f).is_ok());
        assert!(o.query(t, &"x".repeat(65537)).is_err());
        assert_eq!(o.surface().unwrap().query, None);
        o.query(t, "equipe").unwrap();
        assert!(o.surface().unwrap().query_valid);
        assert!(o.prepare(t, Some("reused"), &c, &f).is_ok());
    }
    #[test]
    fn failed_admission_preserves_query_selection_and_allows_new_attempt() {
        let (mut o, c, f, t) = owner();
        o.query(t, "other").unwrap();
        let before = o.surface().unwrap();
        let request = o.prepare(t, Some("two"), &c, &f).unwrap();
        assert!(o
            .admit(request, &c, &f, |_, _, _| Err("capacity".into()))
            .is_err());
        assert_eq!(o.surface().unwrap(), before);
        let r = o.prepare(t, Some("two"), &c, &f).unwrap();
        o.admit(r, &c, &f, |_, _, _| Ok(())).unwrap();
    }
    #[test]
    fn busy_rejects_query_select_move_and_second_admission_but_back_works() {
        let (mut o, c, f, t) = owner();
        let second = o.prepare(t, None, &c, &f).unwrap();
        let origin = pending(&mut o, &c, &f, t);
        assert!(o.surface().unwrap().busy);
        assert!(o.query(t, "new").is_err());
        assert!(!o.select(t, "two"));
        assert!(!o.move_selection(t, 1));
        assert!(o.prepare(t, None, &c, &f).is_err());
        assert!(o
            .admit(second, &c, &f, |_, _, _| panic!("duplicate admission"))
            .is_err());
        assert!(o.back(t));
        assert_eq!(
            o.complete(&origin, choices("late"), None).unwrap(),
            Completion::Stale
        );
        assert!(o.surface().is_none());
    }
    #[test]
    fn failure_retains_query_selection_and_updates_readable_detail() {
        let (mut o, c, f, t) = owner();
        o.query(t, "road").unwrap();
        let origin = pending(&mut o, &c, &f, t);
        assert_eq!(
            o.complete(
                &origin,
                result(json!({"ok":false,"error":"Provider failed","detail":"Failure report"})),
                None
            )
            .unwrap(),
            Completion::Kept
        );
        let s = o.surface().unwrap();
        assert_eq!(s.token, t);
        assert_eq!(s.query.as_deref(), Some("road"));
        assert_eq!(s.selection, Some(0));
        assert_eq!(s.detail.as_deref(), Some("Failure report"));
        assert_eq!(s.error, "Provider failed");
        assert!(!s.busy);
        assert!(o.prepare(t, Some("reused"), &c, &f).is_ok());
    }
    #[test]
    fn success_replaces_with_fresh_token_and_resets_query_even_for_reused_ids() {
        let (mut o, c, f, t) = owner();
        o.query(t, "road").unwrap();
        let origin = pending(&mut o, &c, &f, t);
        assert_eq!(
            o.complete(&origin, choices("REPLACED"), Some("Usage not saved"))
                .unwrap(),
            Completion::Replaced
        );
        let s = o.surface().unwrap();
        assert_ne!(s.token, t);
        assert_eq!(s.query.as_deref(), Some(""));
        assert_eq!(s.rows.len(), 2);
        assert_eq!(s.selection, Some(0));
        assert_eq!(s.error, "Usage not saved");
        assert_eq!(
            o.complete(&origin, choices("duplicate completion"), None)
                .unwrap(),
            Completion::Stale
        );
        let request = o.prepare(s.token, Some("reused"), &c, &f).unwrap();
        o.admit(request, &c, &f, |p, _, _| {
            assert_eq!(p.arguments[0], "REPLACED");
            Ok(())
        })
        .unwrap();
    }
    #[test]
    fn successful_detail_only_is_separate_and_success_without_presentation_closes() {
        let (mut o, c, f, t) = owner();
        let origin = pending(&mut o, &c, &f, t);
        assert_eq!(
            o.complete(
                &origin,
                result(json!({"ok":true,"detail":"Final report"})),
                None
            )
            .unwrap(),
            Completion::Replaced
        );
        let s = o.surface().unwrap();
        assert!(s.rows.is_empty());
        assert_eq!(s.detail.as_deref(), Some("Final report"));
        assert!(o.prepare(s.token, Some("reused"), &c, &f).is_err());
        assert!(o.prepare(s.token, None, &c, &f).is_err());
        o.present(&c, &f, choices("next"), None).unwrap();
        let r = o.prepare(o.surface().unwrap().token, None, &c, &f).unwrap();
        let origin = o.admit(r, &c, &f, |_, _, _| Ok(())).unwrap();
        assert_eq!(
            o.complete(&origin, result(json!({"ok":true})), Some("Usage not saved"))
                .unwrap(),
            Completion::Closed {
                notice: "Usage not saved".into()
            }
        );
        assert!(o.surface().is_none());
    }
    #[test]
    fn late_origin_cannot_overwrite_new_session_with_same_command_and_ids() {
        let (mut o, c, f, t) = owner();
        let origin = pending(&mut o, &c, &f, t);
        o.present(&c, &f, choices("new"), None).unwrap();
        let before = o.surface().unwrap();
        assert_eq!(
            o.complete(&origin, result(json!({"ok":true})), None)
                .unwrap(),
            Completion::Stale
        );
        assert_eq!(o.surface().unwrap(), before);
    }
    #[test]
    fn detail_on_initial_failure_does_not_claim_to_close_an_input_draft() {
        let mut o = Owner::default();
        let c = command();
        let f = Path::new("/own/config.json");
        assert!(o
            .present(
                &c,
                f,
                result(json!({"ok":false,"error":"Failure","detail":"Report"})),
                None
            )
            .unwrap());
        let s = o.surface().unwrap();
        assert!(s.rows.is_empty());
        assert_eq!(s.error, "Failure");
        assert_eq!(s.detail.as_deref(), Some("Report"));
        assert!(o.back(s.token));
    }
    #[test]
    fn plain_result_does_not_replace_existing_presentation() {
        let (mut o, c, f, _) = owner();
        let before = o.surface().unwrap();
        assert!(!o.present(&c, &f, result(json!({"ok":true})), None).unwrap());
        assert_eq!(o.surface().unwrap(), before);
    }
    fn run(command: &Command, home: &Path, required: bool) -> Interpreted {
        let completed = rustcast_core::script::run(command, home, None, || false).unwrap();
        interpret(
            completed.response(),
            required,
            Audience::Native,
            Some(&completed.capture),
        )
    }
    #[test]
    fn real_provider_load_select_failure_reload_replace_and_ok_only_close() {
        let home = std::env::temp_dir().join(format!(
            "ruycast-result-owner-{}",
            objc2::rc::autoreleasepool(|_| objc2_foundation::NSUUID::new()
                .UUIDString()
                .to_string())
        ));
        std::fs::create_dir(&home).unwrap();
        struct Cleanup(PathBuf);
        impl Drop for Cleanup {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
        let _cleanup = Cleanup(home.clone());
        let file = home.join("config.json");
        let marker = home.join("selection.json");
        let phase = home.join("phase");
        let code = r#"import json,sys,pathlib
value=sys.argv[1]; marker=pathlib.Path(sys.argv[2]); phase=pathlib.Path(sys.argv[3])
if not value:
    n=int(phase.read_text())+1 if phase.exists() else 1
    phase.write_text(str(n))
    print(json.dumps(dict(ok=True,choices=dict(title='Projects',placeholder='Find project',items=[dict(id='reused',title='Project',subtitle='Load '+str(n),value='PRIVATE $(literal) Ω '+str(n))]),detail='Load detail '+str(n))))
elif value.endswith(' 1'):
    print(json.dumps(dict(ok=False,error='Not ready',detail='Failure report')))
    sys.exit(7)
else:
    marker.write_text(json.dumps(value));print(json.dumps(dict(ok=True)))
"#;
        let mut c = command();
        c.target = "/usr/bin/python3".into();
        c.arguments = vec![
            "-c".into(),
            code.into(),
            "{input}".into(),
            marker.to_string_lossy().into(),
            phase.to_string_lossy().into(),
        ];
        let initial = crate::execution::prepare_result(&c, None).unwrap();
        let mut o = Owner::default();
        assert!(o
            .present(&c, &file, run(&initial, &home, false), None)
            .unwrap());
        let t = o.surface().unwrap().token;
        o.query(t, "project").unwrap();
        let request = o.prepare(t, Some("reused"), &c, &file).unwrap();
        let mut reply = None;
        let origin = o
            .admit(request, &c, &file, |p, _, origin| {
                reply = Some(run(p, &home, origin.expected_envelope()));
                Ok(())
            })
            .unwrap();
        assert_eq!(
            o.complete(&origin, reply.take().unwrap(), None).unwrap(),
            Completion::Kept
        );
        let s = o.surface().unwrap();
        assert_eq!(s.token, t);
        assert_eq!(s.query.as_deref(), Some("project"));
        assert_eq!(s.error, "Not ready");
        assert_eq!(s.detail.as_deref(), Some("Failure report"));
        assert!(!marker.exists());
        let request = o.prepare(t, None, &c, &file).unwrap();
        let origin = o
            .admit(request, &c, &file, |p, v, origin| {
                assert_eq!(*v, json!({}));
                reply = Some(run(p, &home, origin.expected_envelope()));
                Ok(())
            })
            .unwrap();
        assert_eq!(
            o.complete(&origin, reply.take().unwrap(), Some("Usage write failed"))
                .unwrap(),
            Completion::Replaced
        );
        let s = o.surface().unwrap();
        assert_ne!(s.token, t);
        assert_eq!(s.query.as_deref(), Some(""));
        assert_eq!(s.error, "Usage write failed");
        assert_eq!(s.detail.as_deref(), Some("Load detail 2"));
        assert!(o.prepare(t, Some("reused"), &c, &file).is_err());
        let request = o.prepare(s.token, Some("reused"), &c, &file).unwrap();
        let origin = o
            .admit(request, &c, &file, |p, _, origin| {
                reply = Some(run(p, &home, origin.expected_envelope()));
                Ok(())
            })
            .unwrap();
        let r = reply.take().unwrap();
        assert!(r.envelope);
        assert_eq!(r.response["ok"], true);
        assert_eq!(
            o.complete(&origin, r, None).unwrap(),
            Completion::Closed {
                notice: String::new()
            }
        );
        assert_eq!(
            serde_json::from_str::<String>(&std::fs::read_to_string(marker).unwrap()).unwrap(),
            "PRIVATE $(literal) Ω 2"
        );
        assert!(o.surface().is_none());
    }
    #[test]
    fn required_origin_rejects_real_plain_success_without_replacing_choices() {
        let (mut o, mut c, f, t) = owner();
        // Use an owned initial binding whose selected execution emits ordinary
        // text, which must not be mistaken for a valid follow-up envelope.
        c.target = "/usr/bin/printf".into();
        c.arguments = vec!["ordinary text".into()];
        o.clear();
        o.present(&c, &f, choices("value"), None).unwrap();
        let current = o.surface().unwrap().token;
        assert_ne!(t, current);
        let request = o.prepare(current, Some("reused"), &c, &f).unwrap();
        let mut reply = None;
        let origin = o
            .admit(request, &c, &f, |p, _, origin| {
                reply = Some(run(p, Path::new("/tmp"), origin.expected_envelope()));
                Ok(())
            })
            .unwrap();
        let r = reply.take().unwrap();
        assert_eq!(r.response["ok"], false);
        assert_eq!(r.response["actionCompleted"], true);
        let Completion::Closed { notice } = o.complete(&origin, r, None).unwrap() else {
            panic!("uncertain completed action stayed runnable");
        };
        assert!(notice.contains("invalid response"));
        assert!(o.surface().is_none());
    }
    #[test]
    fn unsafe_capture_cannot_offer_previous_choices_again() {
        for condition in ["missing", "open", "truncated", "utf8"] {
            let (mut o, c, f, t) = owner();
            let origin = pending(&mut o, &c, &f, t);
            let mut capture = rustcast_core::script::Capture::empty();
            match condition {
                "open" => capture.stdout.eof = false,
                "truncated" => capture.stdout.truncated = true,
                "utf8" => capture.stdout.valid_utf8 = false,
                _ => {}
            }
            let proof = (condition != "missing").then_some(&capture);
            let r = interpret(
                json!({"ok":true,"result":{"stdout":"{\"ok\":true}","exitCode":0}}),
                origin.expected_envelope(),
                Audience::Native,
                proof,
            );
            let Completion::Closed { notice } = o.complete(&origin, r, None).unwrap() else {
                panic!("uncertain effect remained runnable: {condition}");
            };
            assert!(notice.contains("incomplete or invalid"));
            assert!(o.surface().is_none());
            assert!(o.prepare(t, Some("reused"), &c, &f).is_err());
        }
    }
    #[test]
    fn completed_failure_detail_is_fresh_readonly_and_preserves_warning() {
        let (mut o, c, f, t) = owner();
        let origin = pending(&mut o, &c, &f, t);
        let r = result(
            json!({"ok":false,"actionCompleted":true,"error":"Check outcome","detail":"Receipt"}),
        );
        assert_eq!(
            o.complete(&origin, r, Some("Usage not saved")).unwrap(),
            Completion::Replaced
        );
        let s = o.surface().unwrap();
        assert_ne!(s.token, t);
        assert!(s.rows.is_empty());
        assert_eq!(s.detail.as_deref(), Some("Receipt"));
        assert_eq!(s.error, "Check outcome\nUsage not saved");
        assert!(o.prepare(s.token, None, &c, &f).is_err());
        assert!(o.prepare(s.token, Some("reused"), &c, &f).is_err());
    }
    #[test]
    fn admission_rechecks_binding_and_file_after_preparation() {
        let (mut o, c, f, t) = owner();
        let before = o.surface().unwrap();
        let r = o.prepare(t, Some("reused"), &c, &f).unwrap();
        let mut changed = c.clone();
        changed.arguments.push("changed".into());
        assert!(o
            .admit(r, &changed, &f, |_, _, _| panic!(
                "changed config reached worker"
            ))
            .is_err());
        assert_eq!(o.surface().unwrap(), before);
        let r = o.prepare(t, None, &c, &f).unwrap();
        assert!(o
            .admit(r, &c, Path::new("/changed/config.json"), |_, _, _| panic!(
                "moved config reached worker"
            ))
            .is_err());
        assert_eq!(o.surface().unwrap(), before);
    }
    #[test]
    fn prepared_intent_from_before_a_failed_job_cannot_run_after_it() {
        let (mut o, c, f, t) = owner();
        let stale = o.prepare(t, Some("reused"), &c, &f).unwrap();
        let origin = pending(&mut o, &c, &f, t);
        assert_eq!(
            o.complete(&origin, result(json!({"ok":false,"error":"Failed"})), None)
                .unwrap(),
            Completion::Kept
        );
        assert!(o
            .admit(stale, &c, &f, |_, _, _| panic!(
                "old queued intent ran after failure"
            ))
            .is_err());
        let fresh = o.prepare(t, Some("reused"), &c, &f).unwrap();
        assert!(o.admit(fresh, &c, &f, |_, _, _| Ok(())).is_ok());
    }
}
