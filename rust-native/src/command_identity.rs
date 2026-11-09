//! Complete Swift Command.Equatable semantics, prepared off native callbacks.
//! Never serialize/debug this identity: target and arguments may be private.
use crate::templates;
use rustcast_core::Command;

#[derive(PartialEq)]
pub struct Identity {
    id: String,
    title: String,
    kind: String,
    target: String,
    aliases: Vec<String>,
    arguments: Vec<String>,
    hotkey: Option<String>,
    timeout: Option<f64>,
    input_prompt: Option<String>,
    keyword: Option<String>,
    application_id: Option<String>,
    snippet_output: Option<String>,
    enabled: Option<bool>,
}
impl Identity {
    pub fn new(command: &Command) -> Self {
        objc2::rc::autoreleasepool(|_| {
            let text = templates::canonical;
            let optional = |v: &Option<String>| v.as_deref().map(text);
            Self {
                id: text(&command.id),
                title: text(&command.title),
                kind: text(&command.kind),
                target: text(&command.target),
                aliases: command.aliases.iter().map(|s| text(s)).collect(),
                arguments: command.arguments.iter().map(|s| text(s)).collect(),
                hotkey: optional(&command.hotkey),
                timeout: command.timeout,
                input_prompt: optional(&command.input_prompt),
                keyword: optional(&command.keyword),
                application_id: optional(&command.application_id),
                snippet_output: optional(&command.snippet_output),
                enabled: command.enabled,
            }
        })
    }
    pub fn id(&self) -> &str {
        &self.id
    }
}
