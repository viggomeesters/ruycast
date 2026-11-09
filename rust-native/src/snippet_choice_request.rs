//! Main-owned choice request transition policy. This supplies no native target
//! or output grant: the native owner must also validate its original Target,
//! Anchor, input observation, secure field and Window.failed before SDK entry.
use crate::command_identity::Identity;
use objc2::MainThreadMarker;
use std::{
    rc::Rc,
    sync::Arc,
    time::{Duration, Instant},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    PendingTrigger,
    Capturing,
    Presenting,
    Visible,
    Returning,
    Applying,
    Finished,
}
/// Classified against retained application identity, never PID alone.
#[derive(Clone, Copy)]
pub enum Activation {
    Original,
    Own,
    Foreign,
    Unknown,
}
#[derive(Clone, Copy)]
pub enum Input {
    OwnFormKey,
    MouseObserved,
    Other,
}
#[derive(Clone)]
pub struct Stamp {
    owner: Rc<()>,
    revision: u64,
    phase: Phase,
}
impl PartialEq for Stamp {
    fn eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.owner, &other.owner)
            && self.revision == other.revision
            && self.phase == other.phase
    }
}
impl Eq for Stamp {}
#[derive(Clone)]
pub struct ConfigTicket {
    owner: Rc<()>,
    serial: u64,
}
pub struct Policy {
    owner: Rc<()>,
    _main: MainThreadMarker,
    original: Arc<Identity>,
    phase: Phase,
    revision: u64,
    activation: u64,
    input: u64,
    generation: Option<u64>,
    last_generation: u64,
    config_ticket: u64,
    due: Instant,
    retries: u8,
    return_activated: bool,
}
impl Policy {
    pub fn new(
        main: MainThreadMarker,
        original: Arc<Identity>,
        now: Instant,
        generation: u64,
        activation: u64,
        input: u64,
    ) -> Self {
        Self {
            owner: Rc::new(()),
            _main: main,
            original,
            phase: if generation == 0 {
                Phase::Finished
            } else {
                Phase::PendingTrigger
            },
            revision: 1,
            activation,
            input,
            generation: (generation != 0).then_some(generation),
            last_generation: generation,
            config_ticket: 0,
            due: now + Duration::from_millis(30),
            retries: 0,
            return_activated: false,
        }
    }
    pub fn phase(&self) -> Phase {
        self.phase
    }
    pub fn deadline(&self) -> Option<Instant> {
        (self.generation.is_some()
            && matches!(self.phase, Phase::PendingTrigger | Phase::Returning))
        .then_some(self.due)
    }
    pub fn stamp(&self) -> Stamp {
        Stamp {
            owner: self.owner.clone(),
            revision: self.revision,
            phase: self.phase,
        }
    }
    pub fn matches(&self, stamp: &Stamp) -> bool {
        self.phase != Phase::Finished && self.stamp() == *stamp
    }
    fn change(&mut self, phase: Phase) {
        self.phase = phase;
        if let Some(next) = self.revision.checked_add(1) {
            self.revision = next;
        } else {
            self.phase = Phase::Finished;
            self.generation = None;
        }
    }
    pub fn cancel(&mut self) {
        self.generation = None;
        self.change(Phase::Finished);
    }
    /// A pending writer revokes effects, while input/activation observation
    /// stays attached. Confirmation can refresh only the config certificate.
    pub fn config_pending(&mut self) -> Option<ConfigTicket> {
        if self.phase == Phase::Finished {
            return None;
        }
        self.generation = None;
        let Some(ticket) = self.config_ticket.checked_add(1) else {
            self.cancel();
            return None;
        };
        self.config_ticket = ticket;
        self.change(self.phase);
        Some(ConfigTicket {
            owner: self.owner.clone(),
            serial: ticket,
        })
    }
    pub fn confirm_config(
        &mut self,
        ticket: &ConfigTicket,
        generation: u64,
        current: Option<&Identity>,
        enabled: bool,
    ) -> bool {
        if self.phase == Phase::Finished {
            return false;
        }
        if !Rc::ptr_eq(&ticket.owner, &self.owner)
            || ticket.serial != self.config_ticket
            || self.generation.is_some()
        {
            return false;
        }
        if generation == 0
            || generation < self.last_generation
            || !enabled
            || current != Some(self.original.as_ref())
        {
            self.cancel();
            return false;
        }
        self.last_generation = generation;
        self.generation = Some(generation);
        self.change(self.phase);
        true
    }
    /// One event at a time, before notification coalescing. Away/back remains
    /// cancelled even if a later foreground sample equals the original app.
    pub fn activated(&mut self, sequence: u64, actor: Activation) {
        if self.phase == Phase::Finished {
            return;
        }
        if self.activation.checked_add(1) != Some(sequence) {
            self.cancel();
            return;
        }
        self.activation = sequence;
        let allowed = matches!(
            (self.phase, actor),
            (Phase::Presenting | Phase::Visible, Activation::Own)
                | (Phase::Returning, Activation::Original | Activation::Own)
        );
        if allowed {
            self.change(self.phase);
        } else {
            self.cancel();
        }
    }
    pub fn input(&mut self, sequence: u64, input: Input) {
        if self.phase == Phase::Finished {
            return;
        }
        if self.input.checked_add(1) != Some(sequence) {
            self.cancel();
            return;
        }
        self.input = sequence;
        if self.phase == Phase::Visible && matches!(input, Input::OwnFormKey | Input::MouseObserved)
        {
            self.change(self.phase);
        } else {
            self.cancel();
        }
    }
    /// Global monitor classification is authoritative; bounds alone cannot
    /// distinguish another application's overlapping window from this form.
    pub fn outside_click(&mut self) {
        self.cancel();
    }
    pub fn begin_capture(&mut self, now: Instant) -> Option<Stamp> {
        if self.phase != Phase::PendingTrigger || now < self.due || self.generation.is_none() {
            return None;
        }
        self.change(Phase::Capturing);
        Some(self.stamp())
    }
    pub fn begin_present(&mut self, capture: &Stamp) -> Option<Stamp> {
        if self.phase != Phase::Capturing || !self.matches(capture) || self.generation.is_none() {
            return None;
        }
        self.change(Phase::Presenting);
        Some(self.stamp())
    }
    /// Expected own activation may have changed the stamp while presenting;
    /// only the still-active Presenting phase can become Visible.
    pub fn presented(&mut self) -> bool {
        if self.phase != Phase::Presenting {
            return false;
        }
        self.change(Phase::Visible);
        true
    }
    /// Establish before orderOut/resign or retained-original activation.
    pub fn begin_return(&mut self, now: Instant) -> Option<Stamp> {
        if self.phase != Phase::Visible || self.generation.is_none() {
            return None;
        }
        self.change(Phase::Returning);
        self.due = now + Duration::from_millis(30);
        self.retries = 0;
        self.return_activated = false;
        Some(self.stamp())
    }
    /// The real owner establishes Returning before hide, then starts the first
    /// 30ms delay only after its one retained-original activation returns.
    pub fn return_activated(&mut self, now: Instant) -> bool {
        if self.phase != Phase::Returning || self.generation.is_none() || self.return_activated {
            return false;
        }
        self.return_activated = true;
        self.due = now + Duration::from_millis(30);
        self.change(Phase::Returning);
        true
    }
    /// Due timing is policy only, never proof that the target/anchor is safe.
    pub fn return_due(&self, now: Instant) -> bool {
        self.phase == Phase::Returning && now >= self.due && self.generation.is_some()
    }
    pub fn retry_own_app(&mut self, now: Instant) -> bool {
        if !self.return_due(now) {
            return false;
        }
        if self.retries == 15 {
            self.cancel();
            return false;
        }
        self.retries += 1;
        self.due = now + Duration::from_millis(30);
        self.change(Phase::Returning);
        true
    }
    /// Native owner calls only AFTER checking original Target/Anchor/security
    /// against this exact stamp. Claim precedes first possible side effect.
    pub fn claim_apply(&mut self, checked: &Stamp, now: Instant) -> Option<Stamp> {
        if !self.return_due(now) || !self.matches(checked) {
            return None;
        }
        self.change(Phase::Applying);
        Some(self.stamp())
    }
    pub fn finish(&mut self) {
        self.cancel();
    }
}

#[cfg(feature = "test-tools")]
pub fn fixture(case: &serde_json::Value) -> Result<serde_json::Value, String> {
    use serde_json::json;
    let main = MainThreadMarker::new().ok_or("Own main fixture required")?;
    let command: rustcast_core::Command =
        serde_json::from_value(case["command"].clone()).map_err(|e| e.to_string())?;
    let mut now = Instant::now();
    let mut policy = Policy::new(main, Arc::new(Identity::new(&command)), now, 1, 0, 0);
    let mut capture = None;
    let mut checked = None;
    let mut tickets = vec![];
    let mut output = vec![];
    for op in case["steps"].as_array().ok_or("Own steps required")? {
        let mut accepted = false;
        match op["op"].as_str() {
            Some("replace") => {
                policy = Policy::new(main, Arc::new(Identity::new(&command)), now, 1, 0, 0);
            }
            Some("advance") => now += Duration::from_millis(op["ms"].as_u64().unwrap_or(30)),
            Some("capture") => {
                let next = policy.begin_capture(now);
                accepted = next.is_some();
                if !op["keep"].as_bool().unwrap_or(false) {
                    capture = next;
                }
            }
            Some("present") => {
                accepted = capture
                    .as_ref()
                    .and_then(|v| policy.begin_present(v))
                    .is_some();
            }
            Some("visible") => accepted = policy.presented(),
            Some("pending") => {
                if let Some(ticket) = policy.config_pending() {
                    tickets.push(ticket);
                }
            }
            Some("config") => {
                let current: Option<rustcast_core::Command> =
                    serde_json::from_value(op["command"].clone()).map_err(|e| e.to_string())?;
                let identity = current.as_ref().map(Identity::new);
                let ticket = if let Some(index) = op["ticket"].as_u64() {
                    tickets.get(index as usize)
                } else {
                    tickets.last()
                };
                if let Some(ticket) = ticket {
                    accepted = policy.confirm_config(
                        ticket,
                        op["generation"].as_u64().unwrap_or(2),
                        identity.as_ref(),
                        op["enabled"].as_bool().unwrap_or(true),
                    );
                }
            }
            Some("activation") => policy.activated(
                op["sequence"].as_u64().unwrap_or(1),
                match op["actor"].as_str() {
                    Some("own") => Activation::Own,
                    Some("original") => Activation::Original,
                    Some("foreign") => Activation::Foreign,
                    _ => Activation::Unknown,
                },
            ),
            Some("input") => policy.input(
                op["sequence"].as_u64().unwrap_or(1),
                match op["kind"].as_str() {
                    Some("form") => Input::OwnFormKey,
                    Some("mouse") => Input::MouseObserved,
                    _ => Input::Other,
                },
            ),
            Some("outside") => policy.outside_click(),
            Some("return") => {
                accepted = policy.begin_return(now).is_some();
            }
            Some("check") => {
                checked = Some(policy.stamp());
            }
            Some("apply") => {
                accepted = checked
                    .as_ref()
                    .and_then(|s| policy.claim_apply(s, now))
                    .is_some();
            }
            Some("retry") => accepted = policy.retry_own_app(now),
            Some("activated-return") => accepted = policy.return_activated(now),
            Some("cancel") => policy.cancel(),
            Some("finish") => policy.finish(),
            _ => return Err("Unknown own policy action".into()),
        }
        output.push(json!({"phase":format!("{:?}",policy.phase()),"accepted":accepted,"returnDue":policy.return_due(now)}));
    }
    Ok(json!(output))
}
