//! Snippet choice form semantics. No activation, AX, input posting or clipboard
//! API: the request owner supplies live context and separately owns effects.
use crate::templates::{self, Context, Field, Limits, Plan};
use objc2_foundation::{NSCharacterSet, NSString};
use rustcast_core::Command;
use std::sync::Arc;

pub struct Prepared {
    command: Command,
    identity: Arc<crate::command_identity::Identity>,
    plan: Plan,
    initial: Vec<String>,
    option_keys: Vec<Vec<String>>,
    initial_nonempty: Vec<bool>,
}
impl Prepared {
    /// Prepare away from a global input callback. Data only after construction.
    pub fn new(command: Command) -> Result<Self, String> {
        objc2::rc::autoreleasepool(|_| {
            let plan = Plan::parse(&command, Limits::application())?;
            let option_keys: Vec<Vec<String>> = plan
                .fields()
                .iter()
                .map(|f| f.options.iter().map(|v| templates::canonical(v)).collect())
                .collect();
            let initial: Vec<String> = plan
                .fields()
                .iter()
                .zip(&option_keys)
                .map(|(f, keys)| {
                    let key = templates::canonical(&f.default_value);
                    if f.options.is_empty() || keys.contains(&key) {
                        f.default_value.clone()
                    } else {
                        f.options[0].clone()
                    }
                })
                .collect();
            let initial_nonempty = initial.iter().map(|v| nonempty(v)).collect();
            Ok(Self {
                identity: Arc::new(crate::command_identity::Identity::new(&command)),
                command,
                plan,
                initial,
                option_keys,
                initial_nonempty,
            })
        })
    }
    pub fn command(&self) -> &Command {
        &self.command
    }
    pub fn identity(&self) -> &Arc<crate::command_identity::Identity> {
        &self.identity
    }
    pub fn fields(&self) -> &[Field] {
        self.plan.fields()
    }
    pub(crate) fn render(
        &self,
        values: &[(String, String)],
        context: &Context<'_>,
    ) -> Result<(String, Option<usize>), String> {
        self.plan.render(values, context)
    }
    pub fn needs_clipboard(&self) -> bool {
        templates::needs_clipboard(&self.command.target)
    }
}
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Action {
    Unhandled,
    Handled,
    Submit,
    Cancel,
}
pub struct Key {
    pub code: u16,
    pub modifiers: u64,
    pub repeat: bool,
    pub editing_text: bool,
    pub composing: bool,
}
pub struct Model {
    prepared: Arc<Prepared>,
    values: Vec<String>,
    active: Option<usize>,
    finished: bool,
    nonempty: Vec<bool>,
}
fn nonempty(value: &str) -> bool {
    NSString::from_str(value)
        .stringByTrimmingCharactersInSet(&NSCharacterSet::whitespaceAndNewlineCharacterSet())
        .length()
        != 0
}
impl Model {
    pub fn new(prepared: Arc<Prepared>) -> Self {
        Self {
            values: prepared.initial.clone(),
            nonempty: prepared.initial_nonempty.clone(),
            active: (!prepared.fields().is_empty()).then_some(0),
            prepared,
            finished: false,
        }
    }
    pub fn prepared(&self) -> &Arc<Prepared> {
        &self.prepared
    }
    pub fn values(&self) -> &[String] {
        &self.values
    }
    pub fn active(&self) -> Option<usize> {
        self.active
    }
    pub fn finished(&self) -> bool {
        self.finished
    }
    pub fn focus(&mut self, field: usize) {
        if field < self.values.len() {
            self.active = Some(field);
        }
    }
    pub fn set(&mut self, field: usize, value: String) -> bool {
        if field >= self.values.len() {
            return false;
        }
        self.active = Some(field);
        self.sync(field, value)
    }
    pub(crate) fn sync(&mut self, field: usize, value: String) -> bool {
        if field >= self.values.len() {
            return false;
        }
        let valid = nonempty(&value);
        self.values[field] = value;
        self.nonempty[field] = valid;
        true
    }
    pub fn can_submit(&self) -> bool {
        self.nonempty.iter().all(|v| *v)
    }
    pub fn selected(&self, field: usize) -> Option<usize> {
        let keys = self.prepared.option_keys.get(field)?;
        let key = templates::canonical(self.values.get(field)?);
        keys.iter().position(|v| *v == key)
    }
    pub fn handle(&mut self, key: Key) -> Action {
        if key.modifiers & ((1 << 17) | (1 << 18) | (1 << 19) | (1 << 20)) != 0 || key.composing {
            return Action::Unhandled;
        }
        match key.code {
            36 | 76 => {
                if key.repeat {
                    Action::Handled
                } else {
                    Action::Submit
                }
            }
            53 => Action::Cancel,
            123..=126 => {
                if key.editing_text {
                    return Action::Unhandled;
                }
                let Some(field) = self.active else {
                    return Action::Unhandled;
                };
                let options = &self.prepared.fields()[field].options;
                if options.is_empty() {
                    return Action::Unhandled;
                }
                if self.finished {
                    return Action::Handled;
                }
                let index = self.selected(field).unwrap_or(0);
                let next = if key.code == 123 || key.code == 126 {
                    index.saturating_sub(1)
                } else {
                    (index + 1).min(options.len() - 1)
                };
                let value = options[next].clone();
                self.set(field, value);
                Action::Handled
            }
            _ => Action::Unhandled,
        }
    }
    /// Validation failure leaves the draft editable. Successful claim precedes
    /// callbacks/native hide so nested events cannot submit it twice.
    pub fn submit(
        &mut self,
        context: &Context<'_>,
    ) -> Result<Option<Vec<(String, String)>>, String> {
        self.submit_if(context, &|| true)
    }
    pub fn submit_if(
        &mut self,
        context: &Context<'_>,
        permit: &impl Fn() -> bool,
    ) -> Result<Option<Vec<(String, String)>>, String> {
        if !permit() {
            return Ok(None);
        }
        if self.finished || !self.can_submit() {
            return Ok(None);
        }
        let values = self
            .prepared
            .fields()
            .iter()
            .zip(&self.values)
            .map(|(f, v)| (f.id.clone(), v.clone()))
            .collect::<Vec<_>>();
        self.prepared.plan.render(&values, context)?;
        if !permit() {
            return Ok(None);
        }
        self.finished = true;
        Ok(Some(values))
    }
    pub fn cancel(&mut self) -> bool {
        if self.finished {
            return false;
        }
        self.finished = true;
        true
    }
}
