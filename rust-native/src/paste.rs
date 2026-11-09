//! Data-only execution/main-thread handoff for launcher/CLI snippet paste.
//! The request context stays alive through delayed phases; native objects and
//! all SDK effects belong to the main-thread controller.
use crate::{execution::Live, snippet_output::Payload};
use serde_json::{json, Value};
use std::sync::{
    atomic::{AtomicBool, AtomicU8, AtomicUsize, Ordering},
    mpsc, Arc, Mutex,
};
use std::time::Instant;

pub const LIMIT: usize = 16;
static TICKETS: AtomicUsize = AtomicUsize::new(0);
pub fn pending_tickets() -> usize {
    TICKETS.load(Ordering::Acquire)
}
struct TicketLease;
impl TicketLease {
    fn acquire() -> Option<Self> {
        TICKETS
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |n| {
                (n < LIMIT).then_some(n + 1)
            })
            .ok()
            .map(|_| Self)
    }
}
impl Drop for TicketLease {
    fn drop(&mut self) {
        TICKETS.fetch_sub(1, Ordering::AcqRel);
    }
}

pub(crate) type WakeSlot = Arc<Mutex<Option<Arc<dyn Fn() + Send + Sync>>>>;
pub struct Prepared {
    pub payload: Payload,
    pub cursor_steps: Option<usize>,
    pub clipboard: Option<crate::clipboard_history::Payload>,
    pub copy_only: bool,
    pub concealed: bool,
}
impl Prepared {
    pub fn clipboard(payload:crate::clipboard_history::Payload,copy_only:bool)->Self{Self{payload:Payload{text:String::new(),html:None,cursor:None},cursor_steps:None,clipboard:Some(payload),copy_only,concealed:false}}
    pub fn utility(text:String,secret:bool,copy_only:bool)->Self{Self{payload:Payload{text,html:None,cursor:None},cursor_steps:None,clipboard:None,copy_only,concealed:secret}}
    pub fn render(
        command: &rustcast_core::Command,
        values: &Value,
        clipboard: Option<&str>,
    ) -> Result<Self, String> {
        if command.kind != "snippet" {
            return Err("Command is not a snippet.".into());
        }
        objc2::rc::autoreleasepool(|_| {
            if let Some(prompt) = &command.input_prompt {
                use objc2_foundation::{NSCharacterSet, NSString};
                if NSString::from_str(values["input"].as_str().unwrap_or(""))
                    .stringByTrimmingCharactersInSet(
                        &NSCharacterSet::whitespaceAndNewlineCharacterSet(),
                    )
                    .is_empty()
                {
                    return Err(format!("Enter {prompt}."));
                }
            }
            let data = values
                .as_object()
                .ok_or("Input values must be strings.")?
                .iter()
                .map(|(k, v)| {
                    v.as_str()
                        .map(|v| (k.clone(), v.to_string()))
                        .ok_or("Input values must be strings.")
                })
                .collect::<Result<Vec<_>, _>>()?;
            let plan =
                crate::templates::Plan::parse(command, crate::templates::Limits::application())?;
            let (text, cursor) =
                plan.render(&data, &crate::templates::Context::current(clipboard))?;
            let payload = crate::snippet_output::render(
                &text,
                command.snippet_output.as_deref(),
                cursor.map(|c| c as isize),
            );
            let cursor_steps = payload
                .cursor
                .map(|c| usize::try_from(c).map_err(|_| "Invalid snippet cursor."))
                .transpose()?
                .map(|c| crate::snippet_output::cursor_steps(&payload.text, c).min(10000));
            Ok(Self {
                payload,
                cursor_steps,
                clipboard: None,
                copy_only: false,
                concealed: false,
            })
        })
    }
}
#[derive(Clone)]
pub struct Outcome {
    pub response: Value,
    pub committed: bool,
}
impl Outcome {
    pub fn success() -> Self {
        Self {
            response: json!({"ok":true,"result":{"state":"pasted"}}),
            committed: true,
        }
    }
    pub fn failure(error: impl Into<String>, committed: bool) -> Self {
        let mut response = json!({"ok":false,"error":error.into()});
        if committed {
            response["actionCompleted"] = json!(true);
            response["code"] = json!("snippet-outcome-unknown");
        }
        Self {
            response,
            committed,
        }
    }
}
#[derive(Default)]
pub struct Dispatch(AtomicU8, Mutex<Option<Outcome>>); // 0 preparing; 1 may mutate clipboard; 2 retired before; 3 retired after.
impl Dispatch {
    pub fn preparing(&self) -> bool {
        self.0.load(Ordering::Acquire) == 0
    }
    pub fn enter(&self) -> bool {
        self.0
            .compare_exchange(0, 1, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
    }
    pub fn live(&self) -> bool {
        self.0.load(Ordering::Acquire) < 2
    }
    pub fn committed(&self) -> bool {
        matches!(self.0.load(Ordering::Acquire), 1 | 3)
    }
    pub fn complete(&self, outcome: &Outcome) {
        let mut ready = self.1.lock().unwrap_or_else(|p| p.into_inner());
        if self.live() {
            *ready = Some(outcome.clone());
        }
    }
    pub fn retire(&self) -> Outcome {
        let ready = self.1.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(outcome) = ready.as_ref() {
            return outcome.clone();
        }
        let mut phase = self.0.load(Ordering::Acquire);
        loop {
            let next = if phase == 0 {
                2
            } else if phase == 1 {
                3
            } else {
                phase
            };
            match self
                .0
                .compare_exchange(phase, next, Ordering::AcqRel, Ordering::Acquire)
            {
                Ok(_) => {
                    return Outcome::failure(
                        if next == 2 {
                            "Request expired or was cancelled."
                        } else {
                            "The snippet may have changed the clipboard or posted input. Check its outcome before trying again."
                        },
                        next == 3,
                    )
                }
                Err(current) => phase = current,
            }
        }
    }
}
pub type HideSlot = Arc<Mutex<Option<crate::connected::HideToken>>>;
enum Event {
    Grant(bool),
    Done(Outcome),
}
pub struct Authority {
    pub deadline: Option<Instant>,
    live: Live,
    revoked: Arc<AtomicBool>,
    pub dispatch: Arc<Dispatch>,
    grant: Mutex<Option<Result<(), String>>>,
}
impl Authority {
    pub fn live(&self) -> bool {
        self.dispatch.live()
            && !self.revoked.load(Ordering::Acquire)
            && !crate::deadline::expired(self.deadline)
            && (self.live)()
    }
    pub fn take_grant(&self) -> Option<Result<(), String>> {
        self.grant.lock().unwrap_or_else(|p| p.into_inner()).take()
    }
}
pub struct Ticket {
    _lease: TicketLease,
    pub prepared: Prepared,
    pub hide: HideSlot,
    pub authority: Arc<Authority>,
    reply: mpsc::SyncSender<Event>,
}
impl Ticket {
    pub fn request_grant(&self, hide: bool) -> bool {
        self.reply.try_send(Event::Grant(hide)).is_ok()
    }
    pub fn finish(self, outcome: Outcome) {
        self.authority.dispatch.complete(&outcome);
        let _ = self.reply.try_send(Event::Done(outcome));
    }
}
#[derive(Clone)]
pub struct Client {
    sender: mpsc::SyncSender<Ticket>,
    wake: WakeSlot,
}
pub struct Server {
    receiver: mpsc::Receiver<Ticket>,
}
pub(crate) fn channel(wake: WakeSlot) -> (Client, Server) {
    let (sender, receiver) = mpsc::sync_channel(LIMIT);
    (Client { sender, wake }, Server { receiver })
}
impl Server {
    pub fn take(&self) -> Vec<Ticket> {
        self.receiver.try_iter().take(LIMIT).collect()
    }
}
impl Client {
    pub fn notify(&self) {
        if let Some(wake) = self.wake.lock().unwrap_or_else(|p| p.into_inner()).clone() {
            wake();
        }
    }
    #[allow(clippy::too_many_arguments)]
    pub fn execute(
        &self,
        prepared: Prepared,
        hide: HideSlot,
        live: Live,
        deadline: Option<Instant>,
        revoked: Arc<AtomicBool>,
        dispatch: Arc<Dispatch>,
        mut grant: impl FnMut(bool) -> Result<(), String>,
    ) -> Outcome {
        struct WakeOnExit<'a>(&'a Client);
        impl Drop for WakeOnExit<'_> {
            fn drop(&mut self) {
                self.0.notify();
            }
        }
        let _cleanup = WakeOnExit(self);
        let authority = Arc::new(Authority {
            live,
            deadline,
            revoked,
            dispatch: dispatch.clone(),
            grant: Mutex::new(None),
        });
        if !authority.live() {
            return dispatch.retire();
        }
        let Some(lease) = TicketLease::acquire() else {
            return Outcome::failure(
                "Snippet paste is busy. Try again after an active command finishes.",
                false,
            );
        };
        let (reply, receive) = mpsc::sync_channel(1);
        let ticket = Ticket {
            _lease: lease,
            prepared,
            hide,
            authority: authority.clone(),
            reply,
        };
        if self.sender.try_send(ticket).is_err() {
            return Outcome::failure(
                "Snippet paste is busy or stopped. Try again after an active command finishes.",
                false,
            );
        }
        self.notify();
        loop {
            // A complete known native result wins simultaneous retirement.
            let event = match receive.try_recv() {
                Ok(event) => Some(event),
                Err(mpsc::TryRecvError::Disconnected) => return dispatch.retire(),
                Err(mpsc::TryRecvError::Empty) => None,
            };
            if let Some(Event::Done(outcome)) = event {
                return outcome;
            }
            if !authority.live() {
                return dispatch.retire();
            }
            if let Some(Event::Grant(hide)) = event {
                let answer = grant(hide);
                *authority.grant.lock().unwrap_or_else(|p| p.into_inner()) = Some(answer);
                self.notify();
                continue;
            }
            match receive.recv_timeout(crate::deadline::poll(deadline)) {
                Ok(Event::Done(outcome)) => return outcome,
                Ok(Event::Grant(hide)) => {
                    let answer = if authority.live() {
                        grant(hide)
                    } else {
                        Err("Request expired or was cancelled.".into())
                    };
                    *authority.grant.lock().unwrap_or_else(|p| p.into_inner()) = Some(answer);
                    self.notify();
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => return dispatch.retire(),
                Err(mpsc::RecvTimeoutError::Timeout) => {}
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn retirement_preserves_known_completion_and_partial_effect_truth() {
        let before = Dispatch::default();
        assert!(!before.retire().committed);
        assert!(!before.enter());
        let after = Dispatch::default();
        assert!(after.enter());
        assert!(after.retire().committed);
        after.complete(&Outcome::success());
        assert_eq!(after.retire().response["ok"], false);
        let ready = Dispatch::default();
        assert!(ready.enter());
        ready.complete(&Outcome::success());
        assert_eq!(ready.retire().response, Outcome::success().response);
    }
    #[test]
    fn retired_worker_does_not_free_main_owned_capacity_and_grants_complete() {
        let wake = Arc::new(Mutex::new(None));
        let (client, server) = channel(wake);
        let live = Arc::new(AtomicBool::new(true));
        let check = live.clone();
        let worker_client = client.clone();
        let worker = std::thread::spawn(move || {
            worker_client.execute(
                Prepared {
                    payload: Payload {
                        text: "own fixture".into(),
                        html: None,
                        cursor: None,
                    },
                    cursor_steps: None,
                    clipboard: None,
                    copy_only: false,
                    concealed: false,
                },
                Arc::default(),
                Arc::new(move || check.load(Ordering::Acquire)),
                Some(Instant::now() + std::time::Duration::from_secs(2)),
                Arc::new(AtomicBool::new(false)),
                Arc::default(),
                |_| Ok(()),
            )
        });
        let ticket = loop {
            if let Some(ticket) = server.take().pop() {
                break ticket;
            }
            assert!(!worker.is_finished());
            std::thread::yield_now();
        };
        assert!(ticket.request_grant(false));
        while ticket.authority.take_grant().is_none() {
            std::thread::yield_now();
        }
        live.store(false, Ordering::Release);
        assert!(!worker.join().unwrap().committed);
        assert_eq!(pending_tickets(), 1);
        let leases = (1..LIMIT)
            .map(|_| TicketLease::acquire().unwrap())
            .collect::<Vec<_>>();
        assert!(TicketLease::acquire().is_none());
        drop(leases);
        drop(ticket);
        assert_eq!(pending_tickets(), 0);
        assert!(TicketLease::acquire().is_some());
    }
}
