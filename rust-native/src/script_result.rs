//! Bounded provider envelopes and worker-owned choice sessions. Native views
//! receive labels/IDs only; executable bindings and literal values stay here.
use rustcast_core::Command;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{collections::HashSet, path::PathBuf};

const INVALID: &str = "The command returned invalid choices. Reload and try again.";
const INVALID_RESPONSE: &str =
    "The app returned an invalid response. Check its current state before retrying.";
const MAX_TEXT: usize = 1_048_576;
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Choice {
    pub id: String,
    pub title: String,
    pub subtitle: String,
    pub value: String,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Choices {
    pub title: String,
    pub placeholder: String,
    pub items: Vec<Choice>,
}
impl Choices {
    pub fn validate(&self) -> Result<(), String> {
        objc2::rc::autoreleasepool(|_| self.validate_in_pool())
    }
    fn validate_in_pool(&self) -> Result<(), String> {
        if self.items.len() > 2000 {
            return Err(INVALID.into());
        }
        let mut seen = HashSet::with_capacity(self.items.len());
        let mut size = self
            .title
            .len()
            .checked_add(self.placeholder.len())
            .ok_or(INVALID)?;
        for item in &self.items {
            if item.id.is_empty() || item.title.is_empty() || item.value.len() > 65536 {
                return Err(INVALID.into());
            }
            for text in [&item.id, &item.title, &item.subtitle, &item.value] {
                size = size
                    .checked_add(text.len())
                    .filter(|s| *s <= MAX_TEXT)
                    .ok_or(INVALID)?;
            }
            // Swift Set<String> treats canonically equivalent IDs as equal.
            // Normalize only the uniqueness key, never provider IDs or values.
            let key = if item.id.is_ascii() {
                std::borrow::Cow::Borrowed(item.id.as_str())
            } else {
                std::borrow::Cow::Owned(
                    objc2_foundation::NSString::from_str(&item.id)
                        .precomposedStringWithCanonicalMapping()
                        .to_string(),
                )
            };
            if !seen.insert(key) {
                return Err(INVALID.into());
            }
        }
        if size > MAX_TEXT {
            return Err(INVALID.into());
        }
        Ok(())
    }
}

#[derive(Debug)]
pub struct Interpreted {
    pub response: Value,
    pub choices: Option<Choices>,
    pub detail: Option<String>,
    pub envelope: bool,
}
/// CLI transports provider metadata; native presentation validates its choices.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Audience {
    Cli,
    Native,
}

/// Bound to the immutable admitted command, never inferred from a provider's
/// fields or a later selection. The optional adapter stays outside the app.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Contract {
    Ordinary,
    Provider,
    Capture,
}
impl Contract {
    pub fn for_command(command: &Command) -> Self {
        if command
            .arguments
            .iter()
            .any(|a| a.ends_with("personal_bridge.py"))
        {
            Self::Provider
        } else if command.id.starts_with("capture.") {
            Self::Capture
        } else {
            Self::Ordinary
        }
    }
}

pub fn interpret_command(
    command: &Command,
    process: Value,
    required: bool,
    audience: Audience,
    capture: Option<&rustcast_core::script::Capture>,
) -> Interpreted {
    let contract = Contract::for_command(command);
    // Private application-level responses are authoritative in CLI as well as
    // native mode. Never accept complete-looking text from an incomplete pipe.
    if contract != Contract::Ordinary
        && capture.is_none_or(|c| !c.stdout.eof || c.stdout.truncated || !c.stdout.valid_utf8)
    {
        return failed_private(
            process,
            "Script output is incomplete or invalid. Check its outcome before retrying.",
        );
    }
    // Existing UI envelopes and bound follow-ups take precedence over capture
    // decoding, including commands whose IDs begin with capture.*.
    let interpreted = interpret(
        process,
        required || contract == Contract::Provider,
        audience,
        capture,
    );
    if contract != Contract::Capture || required || interpreted.envelope {
        return interpreted;
    }
    let process = interpreted.response;
    // Process failure/unknown outcome must not be overwritten by stdout.
    if process["ok"] != true {
        return Interpreted {
            response: process,
            choices: None,
            detail: None,
            envelope: false,
        };
    }
    let output = process["result"]["stdout"]
        .as_str()
        .or_else(|| process["stdout"].as_str());
    let decoded = output
        .filter(|s| s.len() <= MAX_TEXT)
        .and_then(|s| serde_json::from_str::<Value>(s).ok())
        .filter(Value::is_object);
    let Some(decoded) = decoded else {
        return failed_private(
            process,
            "Capture returned an invalid response. Check the inbox before retrying.",
        );
    };
    let response = if decoded["ok"] != true {
        json!({"ok":false,"error":decoded["error"].as_str().unwrap_or("Capture failed.")})
    } else if let Some(error) = decoded["daily_error"].as_str().filter(|s| !s.is_empty()) {
        json!({"ok":false,"error":format!("Saved · daily link failed: {error}"),"actionCompleted":true})
    } else {
        json!({"ok":true,"result":decoded})
    };
    Interpreted {
        response,
        choices: None,
        detail: None,
        envelope: false,
    }
}

fn failed_private(mut process: Value, error: &str) -> Interpreted {
    if process["ok"] == true {
        let code = process["result"]
            .get("exitCode")
            .or_else(|| process.get("exitCode"))
            .cloned();
        process = json!({"ok":false,"error":error,"actionCompleted":true});
        if let Some(code) = code {
            process["exitCode"] = code;
        }
    }
    Interpreted {
        response: process,
        choices: None,
        detail: None,
        envelope: false,
    }
}

/// Only the owner may admit this optional second effect. CLI never calls it.
/// URLComponents encodes the path as data, including '&', '#', '+' and Unicode.
pub fn capture_open_request(
    command: &Command,
    response: &Value,
) -> Result<Option<crate::workspace::OpenRequest>, String> {
    if Contract::for_command(command) != Contract::Capture
        || command.id != "capture.quick-note"
        || response["ok"] != true
    {
        return Ok(None);
    }
    let Some(path) = response["result"]["note_path"].as_str() else {
        return Ok(None);
    };
    if path.contains('\0') {
        return Err("Note saved · could not open Obsidian.".into());
    }
    let target = objc2::rc::autoreleasepool(|_| {
        use objc2_foundation::{NSArray, NSString, NSURLComponents, NSURLQueryItem};
        let parts = NSURLComponents::new();
        parts.setScheme(Some(&NSString::from_str("obsidian")));
        parts.setHost(Some(&NSString::from_str("open")));
        let item = NSURLQueryItem::queryItemWithName_value(
            &NSString::from_str("path"),
            Some(&NSString::from_str(path)),
        );
        parts.setQueryItems(Some(&NSArray::from_slice(&[&*item])));
        parts
            .URL()
            .and_then(|url| url.absoluteString())
            .map(|s| s.to_string())
    })
    .ok_or("Note saved · could not open Obsidian.")?;
    Ok(Some(crate::workspace::OpenRequest {
        kind: crate::workspace::Kind::Url,
        command_id: command.id.clone(),
        title: command.title.clone(),
        target,
        application_id: None,
    }))
}

pub fn capture_open_finished(mut saved: Value, opened: &Value) -> Value {
    if opened["ok"] != true {
        saved["ok"] = json!(false);
        saved["error"] =
            json!("Note saved · could not open Obsidian. Check its state before trying again.");
        saved["actionCompleted"] = json!(true);
        if let Some(code) = opened.get("code") {
            saved["code"] = code.clone();
        }
    }
    saved
}

#[cfg(test)]
mod private_contract_tests {
    use super::*;
    fn command(id: &str, provider: bool) -> Command {
        serde_json::from_value(json!({"id":id,"title":"Owned fixture","kind":"script","target":"/owned/fake","arguments":if provider {vec!["personal_bridge.py"]} else {vec![]}})).unwrap()
    }
    fn capture() -> rustcast_core::script::Capture {
        let mut capture = rustcast_core::script::Capture::empty();
        capture.stdout.eof = true;
        capture.stdout.valid_utf8 = true;
        capture
    }
    #[test]
    fn capture_preserves_ui_envelope_and_bound_followup_precedence() {
        let command = command("capture.owned", false);
        let proof = capture();
        for audience in [Audience::Native, Audience::Cli] {
            for (native, required) in [
                (json!({"ok":true,"detail":"Owned report"}), false),
                (
                    json!({"ok":true,"choices":{"title":"Owned","placeholder":"Select","items":[{"id":"a","title":"A","subtitle":"","value":"literal"}]}}),
                    false,
                ),
                (json!({"ok":true,"cleared":true}), true),
            ] {
                let result = interpret_command(
                    &command,
                    raw(native.clone(), true),
                    required,
                    audience,
                    Some(&proof),
                );
                assert!(
                    result.envelope,
                    "audience={audience:?} required={required} response={:?}",
                    result.response
                );
                assert_eq!(result.response["ok"], true);
                assert!(result.response.get("result").is_none());
                if audience == Audience::Native {
                    assert_eq!(result.detail.as_deref(), native["detail"].as_str());
                    assert_eq!(result.choices.is_some(), native["choices"].is_object());
                }
                let failure = interpret_command(
                    &command,
                    raw(native, false),
                    required,
                    audience,
                    Some(&proof),
                );
                assert_eq!(failure.response["ok"], false);
                assert_eq!(failure.response["error"], "Script exited with status 7.");
                assert!(failure.choices.is_none());
            }
            let invalid = interpret_command(
                &command,
                json!({"ok":true,"result":{"stdout":"not-json","exitCode":0}}),
                true,
                audience,
                Some(&proof),
            );
            assert!(invalid.envelope);
            assert_eq!(invalid.response["error"], INVALID_RESPONSE);
            assert_eq!(invalid.response["actionCompleted"], true);
        }
    }
    fn raw(value: Value, ok: bool) -> Value {
        if ok {
            json!({"ok":true,"result":{"stdout":value.to_string(),"stderr":"","exitCode":0}})
        } else {
            json!({"ok":false,"error":"Script exited with status 7.","stdout":value.to_string(),"stderr":"synthetic-secret-not-logged","exitCode":7})
        }
    }
    #[test]
    fn bound_private_contract_admits_plain_success_and_never_reclassifies_from_response() {
        let provider = command("capture.provider-precedence", true);
        assert_eq!(Contract::for_command(&provider), Contract::Provider);
        crate::execution::eligible(&provider).unwrap();
        for audience in [Audience::Native, Audience::Cli] {
            let result = interpret_command(
                &provider,
                raw(json!({"ok":true,"extra":7}), true),
                false,
                audience,
                Some(&capture()),
            );
            assert!(result.envelope);
            assert_eq!(result.response["ok"], true);
            assert_eq!(result.response["extra"], 7);
        }
        let ordinary = command("ordinary", false);
        let process = raw(json!({"ok":true}), true);
        assert_eq!(
            interpret_command(&ordinary, process.clone(), false, Audience::Cli, None).response,
            process
        );
    }
    #[test]
    fn private_cli_and_native_require_trusted_complete_output_and_preserve_real_failure() {
        for command in [command("capture.fixture", false), command("provider", true)] {
            for audience in [Audience::Native, Audience::Cli] {
                for fault in 0..4 {
                    let mut proof = capture();
                    match fault {
                        0 => proof.stdout.eof = false,
                        1 => proof.stdout.truncated = true,
                        2 => proof.stdout.valid_utf8 = false,
                        _ => {}
                    }
                    let proof = (fault != 3).then_some(&proof);
                    let result = interpret_command(
                        &command,
                        raw(json!({"ok":true,"detail":"forged prefix"}), true),
                        false,
                        audience,
                        proof,
                    );
                    assert_eq!(result.response["ok"], false);
                    assert_eq!(result.response["actionCompleted"], true);
                    assert!(result.detail.is_none());
                    let failure = raw(json!({"ok":true}), false);
                    assert_eq!(
                        interpret_command(&command, failure.clone(), false, audience, proof)
                            .response,
                        failure
                    );
                }
                let result = interpret_command(
                    &command,
                    raw(json!({"ok":true}), false),
                    false,
                    audience,
                    Some(&capture()),
                );
                assert_eq!(result.response["ok"], false);
                assert_eq!(result.response["exitCode"], 7);
            }
        }
    }
    #[test]
    fn capture_application_truth_and_saved_daily_error_are_distinct_from_process_success() {
        let command = command("capture.fixture", false);
        for audience in [Audience::Native, Audience::Cli] {
            let success = interpret_command(
                &command,
                raw(json!({"ok":true,"note_path":"owned"}), true),
                false,
                audience,
                Some(&capture()),
            );
            assert_eq!(success.response["result"]["note_path"], "owned");
            assert!(success.response.get("exitCode").is_none());
            let failure = interpret_command(
                &command,
                raw(json!({"ok":false,"error":"owned failure"}), true),
                false,
                audience,
                Some(&capture()),
            );
            assert_eq!(failure.response["error"], "owned failure");
            assert!(failure.response.get("actionCompleted").is_none());
            let saved = interpret_command(
                &command,
                raw(json!({"ok":true,"daily_error":"owned daily failure"}), true),
                false,
                audience,
                Some(&capture()),
            );
            assert_eq!(saved.response["ok"], false);
            assert_eq!(saved.response["actionCompleted"], true);
            assert!(saved.response["error"].as_str().unwrap().contains("daily"));
            for text in ["not json", "null", "[]", "{\"ok\":true}trailing"] {
                let result = interpret_command(
                    &command,
                    json!({"ok":true,"result":{"stdout":text,"exitCode":0}}),
                    false,
                    audience,
                    Some(&capture()),
                );
                assert_eq!(result.response["ok"], false);
                assert_eq!(result.response["actionCompleted"], true);
            }
        }
    }
    #[test]
    fn quick_note_url_is_data_and_saved_truth_survives_opener_failure() {
        let command = command("capture.quick-note", false);
        let path = "/owned/Note Ω &?# +.md";
        let saved = json!({"ok":true,"result":{"ok":true,"note_path":path}});
        let request = capture_open_request(&command, &saved).unwrap().unwrap();
        objc2::rc::autoreleasepool(|_| {
            use objc2_foundation::{NSString, NSURLComponents};
            let parts = NSURLComponents::componentsWithString(&NSString::from_str(&request.target))
                .unwrap();
            let items = parts.queryItems().unwrap();
            assert_eq!(items.len(), 1);
            assert_eq!(items.objectAtIndex(0).value().unwrap().to_string(), path);
            assert!(parts.fragment().is_none());
            assert_eq!(parts.scheme().unwrap().to_string(), "obsidian");
        });
        assert_eq!(
            capture_open_finished(saved.clone(), &json!({"ok":true})),
            saved
        );
        let failed = capture_open_finished(
            saved,
            &json!({"ok":false,"code":"open-outcome-unknown","error":"sensitive SDK path"}),
        );
        assert_eq!(failed["ok"], false);
        assert_eq!(failed["actionCompleted"], true);
        assert_eq!(failed["code"], "open-outcome-unknown");
        assert!(!failed["error"].as_str().unwrap().contains("sensitive"));
        assert!(
            capture_open_request(&command, &json!({"ok":false,"result":{"note_path":path}}))
                .unwrap()
                .is_none()
        );
        assert!(capture_open_request(
            &command,
            &json!({"ok":true,"result":{"note_path":"bad\0path"}})
        )
        .is_err());
        assert!(capture_open_request(
            &self::command("not-capture", false),
            &json!({"ok":true,"result":{"note_path":path}})
        )
        .unwrap()
        .is_none());
    }
}
/// `required` belongs to the originating job, never to whichever UI happens
/// to exist at completion. In particular a choice follow-up can return only ok.
pub fn interpret(
    process: Value,
    required: bool,
    audience: Audience,
    capture: Option<&rustcast_core::script::Capture>,
) -> Interpreted {
    // Never parse potentially altered/truncated provider text, including a
    // complete-looking JSON prefix. A provider field cannot supply this proof.
    if audience == Audience::Native
        && capture.is_none_or(|c| !c.stdout.eof || c.stdout.truncated || !c.stdout.valid_utf8)
    {
        let mut response = process;
        if response["ok"] == true {
            response["ok"] = json!(false);
            response["actionCompleted"] = json!(true);
            response["error"] =
                json!("Script output is incomplete or invalid. Check its outcome before retrying.");
        }
        return Interpreted {
            response,
            choices: None,
            detail: None,
            envelope: false,
        };
    }
    let output = process
        .get("result")
        .and_then(|v| v.get("stdout"))
        .and_then(Value::as_str)
        .or_else(|| process.get("stdout").and_then(Value::as_str));
    let native = output
        .filter(|s| s.len() <= MAX_TEXT)
        .and_then(|s| serde_json::from_str::<Value>(s).ok());
    let recognized = native.as_ref().is_some_and(|v| {
        v["ok"].is_boolean() && (v["choices"].is_object() || v["detail"].is_string())
    });
    if !required && !recognized {
        return Interpreted {
            response: process,
            choices: None,
            detail: None,
            envelope: false,
        };
    }
    let Some(mut response) = native.filter(|v| v.is_object() && v["ok"].is_boolean()) else {
        // A required envelope does not turn an actual process failure into a
        // presentation failure. Keep its real exit error/status; only a
        // successful process with a missing envelope has an uncertain effect.
        if process["ok"] != true {
            return Interpreted {
                response: process,
                choices: None,
                detail: None,
                envelope: false,
            };
        }
        let mut response = json!({"ok":false,"error":INVALID_RESPONSE});
        if let Some(code) = process.get("exitCode") {
            response["exitCode"] = code.clone();
        }
        if process["ok"] == true {
            response["actionCompleted"] = json!(true);
        }
        return Interpreted {
            response,
            choices: None,
            detail: None,
            envelope: true,
        };
    };
    if let Some(code) = process
        .get("result")
        .and_then(|v| v.get("exitCode"))
        .or_else(|| process.get("exitCode"))
    {
        response["exitCode"] = code.clone();
    }
    // OS process failure wins over a success claim from stdout. Preserve detail
    // and native extra fields for the CLI, without writing payloads to logs.
    if process["ok"] != true && response["ok"] == true {
        response["ok"] = json!(false);
        response["error"] = process
            .get("error")
            .cloned()
            .unwrap_or(json!("Provider process failed."));
    }
    let mut choices = None;
    if audience == Audience::Native && response["ok"] == true {
        if let Some(raw) = response.get("choices") {
            match serde_json::from_value::<Choices>(raw.clone())
                .ok()
                .filter(|c| c.validate().is_ok())
            {
                Some(parsed) => choices = Some(parsed),
                None => {
                    response["ok"] = json!(false);
                    response["error"] = json!(INVALID);
                    response["actionCompleted"] = json!(true);
                }
            }
        }
    }
    let detail = (audience == Audience::Native)
        .then(|| {
            response["detail"]
                .as_str()
                .filter(|s| s.len() <= MAX_TEXT)
                .map(str::to_owned)
        })
        .flatten();
    Interpreted {
        response,
        choices,
        detail,
        envelope: true,
    }
}

/// No serialization: this contains private command arguments and choice values.
pub struct ChoiceSession {
    token: u64,
    command: Command,
    file: PathBuf,
    choices: Choices,
    query: String,
    filtered: Vec<usize>,
    selection: Option<usize>,
    locale: String,
    folded_labels: Vec<String>,
}
impl ChoiceSession {
    pub fn new(
        token: u64,
        command: &Command,
        file: PathBuf,
        choices: Choices,
    ) -> Result<Self, String> {
        if token == 0 || !file.is_absolute() {
            return Err("Invalid script result owner.".into());
        }
        choices.validate()?;
        crate::execution::eligible(command)?;
        crate::execution::budget(command, None)?;
        let filtered: Vec<_> = (0..choices.items.len()).collect();
        let selection = (!filtered.is_empty()).then_some(0);
        let (locale, folded_labels) = objc2::rc::autoreleasepool(|_| {
            let locale = objc2_foundation::NSLocale::currentLocale();
            (
                locale.localeIdentifier().to_string(),
                fold_labels(&choices, &locale),
            )
        });
        Ok(Self {
            token,
            command: command.clone(),
            file,
            choices,
            query: String::new(),
            filtered,
            selection,
            locale,
            folded_labels,
        })
    }
    pub fn token(&self) -> u64 {
        self.token
    }
    pub fn command(&self) -> &Command {
        &self.command
    }
    pub fn file(&self) -> &std::path::Path {
        &self.file
    }
    pub fn choices(&self) -> &Choices {
        &self.choices
    }
    pub fn query(&self) -> &str {
        &self.query
    }
    pub fn selection(&self) -> Option<usize> {
        self.selection
    }
    pub fn set_query(&mut self, query: &str) -> Result<(), String> {
        objc2::rc::autoreleasepool(|_| {
            self.set_query_in_locale(query, &objc2_foundation::NSLocale::currentLocale())
        })
    }
    fn set_query_in_locale(
        &mut self,
        query: &str,
        locale: &objc2_foundation::NSLocale,
    ) -> Result<(), String> {
        if query.len() > 65_536 {
            return Err("Search exceeds the supported size.".into());
        }
        let folded = fold(query, locale);
        let mut terms = HashSet::new();
        for term in folded.split_whitespace() {
            terms.insert(term);
            if terms.len() > 64 {
                return Err("Search contains too many words.".into());
            }
        }
        let id = locale.localeIdentifier().to_string();
        if id != self.locale {
            self.folded_labels = fold_labels(&self.choices, locale);
            self.locale = id;
        }
        self.filtered = self
            .folded_labels
            .iter()
            .enumerate()
            .filter_map(|(i, label)| terms.iter().all(|t| label.contains(t)).then_some(i))
            .collect();
        self.selection = (!self.filtered.is_empty()).then_some(0);
        self.query = query.into();
        Ok(())
    }
    pub fn rows(&self) -> impl Iterator<Item = &Choice> {
        self.filtered.iter().map(|&i| &self.choices.items[i])
    }
    pub fn selected(&self) -> Option<&Choice> {
        self.selection
            .and_then(|i| self.filtered.get(i))
            .map(|&i| &self.choices.items[i])
    }
    pub fn select(&mut self, id: &str) -> bool {
        if let Some(i) = self
            .filtered
            .iter()
            .position(|&i| self.choices.items[i].id == id)
        {
            self.selection = Some(i);
            true
        } else {
            false
        }
    }
    pub fn move_selection(&mut self, delta: isize) {
        self.selection = if self.filtered.is_empty() {
            None
        } else {
            Some(
                self.selection
                    .unwrap_or(0)
                    .saturating_add_signed(delta)
                    .min(self.filtered.len() - 1),
            )
        };
    }
    pub fn value(&self, token: u64, id: &str) -> Option<&str> {
        (token == self.token)
            .then(|| self.selected())
            .flatten()
            .filter(|c| c.id == id)
            .map(|c| c.value.as_str())
    }
    pub fn matches(&self, current: &Command, file: &std::path::Path) -> bool {
        self.file == file
            && crate::actions::binding_identity(&self.command, current)
            && current.enabled != Some(false)
    }
}

fn fold(text: &str, locale: &objc2_foundation::NSLocale) -> String {
    use objc2_foundation::{NSString, NSStringCompareOptions};
    NSString::from_str(text)
        .stringByFoldingWithOptions_locale(
            NSStringCompareOptions::CaseInsensitiveSearch
                | NSStringCompareOptions::DiacriticInsensitiveSearch,
            Some(locale),
        )
        .to_string()
}
fn fold_labels(choices: &Choices, locale: &objc2_foundation::NSLocale) -> Vec<String> {
    choices
        .items
        .iter()
        .map(|c| fold(&format!("{} {}", c.title, c.subtitle), locale))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    fn capture() -> rustcast_core::script::Capture {
        rustcast_core::script::Capture::empty()
    }
    #[test]
    fn required_malformed_envelope_preserves_real_process_failure() {
        for stdout in [
            "ordinary failure text",
            "{invalid",
            "{\"ok\":\"not a boolean\"}",
        ] {
            let process = json!({"ok":false,"error":"Script exited with status 7.","exitCode":7,"stdout":stdout,"stderr":""});
            let r = interpret(
                process.clone(),
                true,
                Audience::Native,
                Some(&rustcast_core::script::Capture::empty()),
            );
            assert_eq!(r.response, process);
            assert!(r.choices.is_none() && r.detail.is_none());
            assert!(!r.envelope);
            assert_ne!(r.response["actionCompleted"], true);
        }
    }
    fn choices() -> Choices {
        Choices {
            title: "Projects".into(),
            placeholder: "Search…".into(),
            items: vec![
                Choice {
                    id: "one".into(),
                    title: "Café Project".into(),
                    subtitle: "EX / PLAN · büro".into(),
                    value: "$(literal)\nΩ\0".into(),
                },
                Choice {
                    id: "two".into(),
                    title: "Other".into(),
                    subtitle: "EX / DEV".into(),
                    value: "not search material".into(),
                },
            ],
        }
    }
    fn process(native: Value, ok: bool) -> Value {
        if ok {
            json!({"ok":true,"result":{"exitCode":0,"stdout":native.to_string(),"stderr":""}})
        } else {
            json!({"ok":false,"exitCode":7,"error":"Script exited with status 7.","stdout":native.to_string(),"stderr":"PRIVATE stderr"})
        }
    }
    #[test]
    fn trusted_capture_precedes_provider_interpretation_and_cannot_be_spoofed() {
        let raw = process(
            json!({"ok":true,"detail":"valid prefix","capture":{"stdout":{"truncated":false,"validUtf8":true}}}),
            true,
        );
        for metadata in [
            None,
            Some(rustcast_core::script::Capture {
                stdout: rustcast_core::script::StreamCapture {
                    truncated: false,
                    eof: false,
                    valid_utf8: true,
                },
                ..capture()
            }),
            Some(rustcast_core::script::Capture {
                stdout: rustcast_core::script::StreamCapture {
                    truncated: true,
                    eof: true,
                    valid_utf8: true,
                },
                ..capture()
            }),
            Some(rustcast_core::script::Capture {
                stdout: rustcast_core::script::StreamCapture {
                    truncated: false,
                    eof: true,
                    valid_utf8: false,
                },
                ..capture()
            }),
        ] {
            let out = interpret(raw.clone(), false, Audience::Native, metadata.as_ref());
            assert_eq!(out.response["ok"], false);
            assert_eq!(out.response["actionCompleted"], true);
            assert!(out.detail.is_none() && out.choices.is_none() && !out.envelope);
            let failed = process(json!({"ok":true,"detail":"do not trust"}), false);
            let out = interpret(failed.clone(), true, Audience::Native, metadata.as_ref());
            assert_eq!(out.response, failed);
            assert!(out.detail.is_none() && !out.envelope);
            let cli = interpret(raw.clone(), false, Audience::Cli, metadata.as_ref());
            assert_eq!(cli.response["ok"], true);
            assert_eq!(cli.response["detail"], "valid prefix");
            assert_eq!(
                cli.response["capture"],
                json!({"stdout":{"truncated":false,"validUtf8":true}})
            );
        }
        let stderr_only = rustcast_core::script::Capture {
            stderr: rustcast_core::script::StreamCapture {
                truncated: true,
                eof: true,
                valid_utf8: false,
            },
            ..capture()
        };
        assert_eq!(
            interpret(raw, false, Audience::Native, Some(&stderr_only))
                .detail
                .as_deref(),
            Some("valid prefix")
        );
    }
    #[test]
    fn provider_passthrough_keeps_extras_and_both_presentations() {
        let native = json!({"ok":true,"choices":choices(),"detail":"Report Ω","extra":{"value":"preserved"}});
        let out = interpret(
            process(native.clone(), true),
            false,
            Audience::Native,
            Some(&capture()),
        );
        assert!(out.envelope);
        assert_eq!(out.choices, Some(choices()));
        assert_eq!(out.detail.as_deref(), Some("Report Ω"));
        let mut expected = native;
        expected["exitCode"] = json!(0);
        assert_eq!(out.response, expected);
    }
    #[test]
    fn process_failure_wins_and_failure_detail_survives_without_accepting_choices() {
        for native_ok in [true, false] {
            let out = interpret(
                process(
                    json!({"ok":native_ok,"error":"Provider error","choices":choices(),"detail":"Failure report"}),
                    false,
                ),
                false,
                Audience::Native,
                Some(&capture()),
            );
            assert_eq!(out.response["ok"], false);
            assert_eq!(out.response["exitCode"], 7);
            assert!(out.choices.is_none());
            assert_eq!(out.detail.as_deref(), Some("Failure report"));
            assert_eq!(
                out.response["error"],
                if native_ok {
                    "Script exited with status 7."
                } else {
                    "Provider error"
                }
            );
        }
    }
    #[test]
    fn followup_contract_is_pinned_in_the_job_not_in_current_global_ui() {
        let raw = process(json!({"ok":true,"extra":7}), true);
        assert!(!interpret(raw.clone(), false, Audience::Native, Some(&capture())).envelope);
        let out = interpret(raw, true, Audience::Native, Some(&capture()));
        assert!(out.envelope);
        assert_eq!(out.response, json!({"ok":true,"extra":7,"exitCode":0}));
        assert!(out.choices.is_none() && out.detail.is_none());
        for text in [
            "not JSON",
            "{\"ok\":true,\"choices\":",
            "null",
            "{}",
            "{\"ok\":1}",
        ] {
            let out = interpret(
                json!({"ok":true,"result":{"stdout":text}}),
                true,
                Audience::Native,
                Some(&capture()),
            );
            assert_eq!(out.response["ok"], false);
            assert_eq!(out.response["actionCompleted"], true);
            assert!(out.choices.is_none());
        }
    }
    #[test]
    fn ordinary_stdout_is_unchanged_and_invalid_choices_are_never_executable() {
        for text in ["hello", "{\"ok\":true}", "{\"detail\":\"ordinary JSON\"}"] {
            let raw = json!({"ok":true,"result":{"stdout":text,"exitCode":0}});
            assert_eq!(
                interpret(raw.clone(), false, Audience::Native, Some(&capture())).response,
                raw
            );
        }
        let mut invalid = choices();
        invalid.items[1].id = invalid.items[0].id.clone();
        let out = interpret(
            process(
                json!({"ok":true,"choices":invalid,"detail":"Keep report"}),
                true,
            ),
            false,
            Audience::Native,
            Some(&capture()),
        );
        assert_eq!(out.response["ok"], false);
        assert!(out.choices.is_none());
        assert_eq!(out.detail.as_deref(), Some("Keep report"));
        assert_eq!(out.response["actionCompleted"], true);
    }
    #[test]
    fn cli_transports_invalid_choices_without_gui_validation_or_duplicate_payloads() {
        let native =
            json!({"ok":true,"choices":{"invalid":true},"detail":"Keep CLI report","extra":17});
        let out = interpret(process(native.clone(), true), false, Audience::Cli, None);
        let mut expected = native;
        expected["exitCode"] = json!(0);
        assert_eq!(out.response, expected);
        assert!(out.envelope);
        assert!(out.choices.is_none() && out.detail.is_none());
    }
    #[test]
    fn search_work_is_bounded_deduplicated_and_locale_cache_is_invalidated() {
        let command: Command = serde_json::from_value(
            json!({"id":"fixture","title":"Fixture","kind":"script","target":"/usr/bin/true"}),
        )
        .unwrap();
        let mut list = choices();
        list.items = (0..2000)
            .map(|i| Choice {
                id: i.to_string(),
                title: "A Café I".into(),
                subtitle: "Plan büro".into(),
                value: "private value".into(),
            })
            .collect();
        let mut s =
            ChoiceSession::new(1, &command, PathBuf::from("/own/config.json"), list).unwrap();
        s.set_query(&"a ".repeat(32768)).unwrap();
        assert_eq!(s.rows().count(), 2000);
        s.select("1");
        let before = s.query.clone();
        assert!(s.set_query(&"a ".repeat(32769)).is_err());
        assert_eq!(s.query, before);
        assert_eq!(s.selected().unwrap().id, "1");
        let words = (0..65)
            .map(|i| format!("term{i}"))
            .collect::<Vec<_>>()
            .join(" ");
        assert!(s.set_query(&words).is_err());
        assert_eq!(s.query, before);
        assert_eq!(s.selected().unwrap().id, "1");
        objc2::rc::autoreleasepool(|_| {
            use objc2_foundation::{NSLocale, NSString};
            for id in ["en_US", "tr_TR", "en_US"] {
                let locale = NSLocale::localeWithLocaleIdentifier(&NSString::from_str(id));
                s.set_query_in_locale("i", &locale).unwrap();
                let expected = s
                    .choices
                    .items
                    .iter()
                    .filter(|c| {
                        fold(&format!("{} {}", c.title, c.subtitle), &locale)
                            .contains(&fold("i", &locale))
                    })
                    .count();
                assert_eq!(s.rows().count(), expected);
                assert_eq!(s.locale, locale.localeIdentifier().to_string());
            }
        });
    }
    #[test]
    fn native_choice_ids_match_swift_canonical_equality_and_preserve_original_bytes() {
        for (a, b) in [
            ("é", "e\u{301}"),
            ("Å", "\u{212b}"),
            ("가", "\u{1100}\u{1161}"),
            ("a\u{315}\u{300}", "a\u{300}\u{315}"),
        ] {
            let mut c = choices();
            c.items[0].id = a.into();
            c.items[1].id = b.into();
            assert!(c.validate().is_err());
            let native = json!({"ok":true,"choices":c});
            assert_eq!(
                interpret(
                    process(native.clone(), true),
                    false,
                    Audience::Native,
                    Some(&capture())
                )
                .response["ok"],
                false
            );
            let cli = interpret(process(native.clone(), true), false, Audience::Cli, None);
            assert_eq!(cli.response["ok"], true);
            assert_eq!(cli.response["choices"], native["choices"]);
        }
        for (a, b) in [("é", "É"), ("a", "A"), ("①", "1")] {
            let mut c = choices();
            c.items[0].id = a.into();
            c.items[1].id = b.into();
            c.items[0].value = "e\u{301}".into();
            c.validate().unwrap();
            let out = interpret(
                process(json!({"ok":true,"choices":c.clone()}), true),
                false,
                Audience::Native,
                Some(&capture()),
            );
            assert_eq!(out.choices, Some(c));
        }
    }
    #[test]
    fn choice_limits_count_bytes_and_preserve_legacy_empty_and_optional_metadata_rules() {
        let mut c = choices();
        c.items[0].value = "Ω".repeat(32768);
        assert!(c.validate().is_ok());
        c.items[0].value.push('x');
        assert!(c.validate().is_err());
        c.items.clear();
        assert!(c.validate().is_ok());
        for (count, ok) in [(2000, true), (2001, false)] {
            c.items = (0..count)
                .map(|i| Choice {
                    id: i.to_string(),
                    title: " ".into(),
                    subtitle: String::new(),
                    value: String::new(),
                })
                .collect();
            assert_eq!(c.validate().is_ok(), ok);
        }
        let raw = json!({"title":"","placeholder":"","items":[],"future":"ignored by baseline"});
        assert!(serde_json::from_value::<Choices>(raw)
            .unwrap()
            .validate()
            .is_ok());
        assert!(serde_json::from_value::<Choices>(
            json!({"title":"Missing placeholder","items":[]})
        )
        .is_err());
    }
    #[test]
    fn selection_filters_labels_in_provider_order_and_binds_literal_value_and_execution() {
        let command:Command=serde_json::from_value(json!({"id":"fixture","title":"Fixture","kind":"script","target":"/usr/bin/true","arguments":["{input}"]})).unwrap();
        let mut s =
            ChoiceSession::new(17, &command, PathBuf::from("/own/config.json"), choices()).unwrap();
        assert_eq!(s.value(17, "one"), Some("$(literal)\nΩ\0"));
        assert!(s.value(16, "one").is_none());
        assert!(s.value(17, "two").is_none());
        s.set_query("CAFE büRO EX").unwrap();
        assert_eq!(
            s.rows().map(|c| c.id.as_str()).collect::<Vec<_>>(),
            vec!["one"]
        );
        s.set_query("not search material").unwrap();
        assert!(s.selected().is_none());
        s.move_selection(1);
        assert!(s.value(17, "two").is_none());
        s.set_query("EX").unwrap();
        s.move_selection(1);
        assert_eq!(s.selected().unwrap().id, "two");
        s.set_query("EX").unwrap();
        assert_eq!(s.selected().unwrap().id, "one");
        assert!(s.select("two"));
        assert!(!s.select("missing"));
        assert_eq!(s.selected().unwrap().id, "two");
        assert!(s.matches(&command, std::path::Path::new("/own/config.json")));
        let mut changed = command.clone();
        changed.timeout = Some(5.);
        assert!(!s.matches(&changed, &s.file));
        assert!(!s.matches(&command, std::path::Path::new("/new/config.json")));
        assert!(ChoiceSession::new(0, &command, s.file.clone(), choices()).is_err());
        assert!(ChoiceSession::new(18, &command, PathBuf::from("relative"), choices()).is_err());
        changed.enabled = Some(false);
        assert!(ChoiceSession::new(18, &changed, s.file.clone(), choices()).is_err());
        s.set_query("cafe buro plan").unwrap();
        assert_eq!(s.selected().unwrap().id, "one");
        s.set_query("EX").unwrap();
        assert!(s.set_query(&"x".repeat(MAX_TEXT + 1)).is_err());
        assert_eq!(s.query, "EX");
    }
}
