//! Main-owned foreground capture / activation requests. The worker's original
//! live gate and absolute deadline are checked immediately before each effect.
use super::*;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    mpsc, Mutex,
};
type Wake = Arc<dyn Fn() + Send + Sync>;
#[cfg(any(test, feature = "test-tools"))]
#[derive(Default)]
struct FixtureAudit {
    required: bool,
    enabled: bool,
    blocked: usize,
    requests: std::collections::BTreeMap<&'static str, usize>,
}
enum Operation {
    Effect(crate::system_actions::Effect),
    Foreground,
    Activate(Identity),
    Screens,
}
enum Reply {
    Foreground(Option<Application>),
    Activated,
    Screens(Vec<rustcast_core::Frame>),
}
struct Request {
    op: Operation,
    ticket: Ticket,
    reply: mpsc::SyncSender<Result<Reply, String>>,
}
#[derive(Clone)]
pub struct Client {
    #[cfg(any(test, feature = "test-tools"))]
    audit: Arc<Mutex<FixtureAudit>>,
    send: mpsc::SyncSender<Request>,
    wake: Arc<Mutex<Option<Wake>>>,
}
pub struct Main {
    #[cfg(any(test, feature = "test-tools"))]
    audit: Arc<Mutex<FixtureAudit>>,
    receive: mpsc::Receiver<Request>,
    foreground: native::Foreground,
    wake: Arc<Mutex<Option<Wake>>>,
    fixture: AtomicBool,
}
pub fn channel() -> (Client, Main) {
    #[cfg(any(test, feature = "test-tools"))]
    let audit = Arc::new(Mutex::new(FixtureAudit::default()));
    let (send, receive) = mpsc::sync_channel(8);
    let wake = Arc::new(Mutex::new(None));
    (
        Client {
            #[cfg(any(test, feature = "test-tools"))]
            audit: audit.clone(),
            send,
            wake: wake.clone(),
        },
        Main {
            #[cfg(any(test, feature = "test-tools"))]
            audit,
            receive,
            foreground: Default::default(),
            wake,
            fixture: AtomicBool::new(false),
        },
    )
}
impl Client {
    #[cfg(any(test, feature = "test-tools"))]
    pub fn fixture_audit(&self) -> serde_json::Value {
        let a = self.audit.lock().unwrap_or_else(|p| p.into_inner());
        serde_json::json!({"required":a.required,"enabled":a.enabled,"blocked":a.blocked,"requests":a.requests})
    }
    pub fn effect(&self, effect: crate::system_actions::Effect, t: &Ticket) -> Result<(), String> {
        match self.request(Operation::Effect(effect), t)? {
            Reply::Activated => Ok(()),
            _ => Err("Invalid system action reply.".into()),
        }
    }
    pub fn screens(&self, t: &Ticket) -> Result<Vec<rustcast_core::Frame>, String> {
        match self.request(Operation::Screens, t)? {
            Reply::Screens(s) => Ok(s),
            _ => Err("Invalid screen reply.".into()),
        }
    }
    pub fn attach_wake(&self, wake: Wake) {
        *self.wake.lock().unwrap_or_else(|p| p.into_inner()) = Some(wake.clone());
        wake();
    }
    fn request(&self, op: Operation, t: &Ticket) -> Result<Reply, String> {
        t.check()?;
        let (send, receive) = mpsc::sync_channel(1);
        self.send
            .try_send(Request {
                op,
                ticket: t.clone(),
                reply: send,
            })
            .map_err(|_| "Native system service is busy or stopped.")?;
        if let Some(wake) = self.wake.lock().unwrap_or_else(|p| p.into_inner()).clone() {
            wake();
        }
        loop {
            t.check()?;
            match receive.recv_timeout(crate::deadline::poll(Some(t.deadline))) {
                Ok(r) => {
                    t.check()?;
                    return r;
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(_) => return Err("Native system service stopped.".into()),
            }
        }
    }
    pub fn foreground(&self, t: &Ticket) -> Result<Option<Application>, String> {
        match self.request(Operation::Foreground, t)? {
            Reply::Foreground(a) => Ok(a),
            _ => Err("Invalid native system reply.".into()),
        }
    }
    pub fn activate(&self, id: Identity, t: &Ticket) -> Result<(), String> {
        match self.request(Operation::Activate(id), t)? {
            Reply::Activated => Ok(()),
            _ => Err("Invalid native system reply.".into()),
        }
    }
}
impl Main {
    /// Independent test guard: an omitted fixture initialization must fail
    /// before any production SDK closure, never fall through to the desktop.
    #[cfg(any(test, feature = "test-tools"))]
    pub fn require_fixture(&self, value: bool) {
        self.audit
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .required = value;
    }
    #[cfg(any(test, feature = "test-tools"))]
    fn fixture_guard(&self) -> bool {
        let mut a = self.audit.lock().unwrap_or_else(|p| p.into_inner());
        if a.required && !self.fixture.load(Ordering::Acquire) {
            a.blocked += 1;
            false
        } else {
            true
        }
    }
    pub fn fixture(&self, value: bool) {
        self.fixture.store(value, Ordering::Release);
        #[cfg(any(test, feature = "test-tools"))]
        {
            self.audit.lock().unwrap_or_else(|p| p.into_inner()).enabled = value;
        }
    }
    pub fn attach_wake(&self, wake: Wake) {
        *self.wake.lock().unwrap_or_else(|p| p.into_inner()) = Some(wake.clone());
        wake();
    }
    pub fn remember(&self) {
        #[cfg(any(test, feature = "test-tools"))]
        if !self.fixture_guard() {
            return;
        }
        self.foreground
            .remember(self.fixture.load(Ordering::Acquire));
    }
    pub fn drain(&self) {
        if objc2::MainThreadMarker::new().is_none() {
            return;
        }
        let fixture = self.fixture.load(Ordering::Acquire);
        self.drain_complete(
            || {
                self.foreground.remember(fixture);
                Ok(self.foreground.snapshot())
            },
            |id, t| {
                if fixture {
                    Err("Native activation is disabled in fixtures.".into())
                } else {
                    native::activate(id, t)
                }
            },
            || {
                if fixture {
                    return Ok(vec![
                        rustcast_core::Frame {
                            x: 0.,
                            y: 0.,
                            width: 1200.,
                            height: 800.,
                        },
                        rustcast_core::Frame {
                            x: 1200.,
                            y: 0.,
                            width: 1000.,
                            height: 700.,
                        },
                    ]);
                }
                let main =
                    objc2::MainThreadMarker::new().ok_or("Screens require the main thread.")?;
                let screens = objc2_app_kit::NSScreen::screens(main);
                if screens.is_empty() || screens.len() > 64 {
                    return Err("No screen available.".into());
                }
                let top = screens.objectAtIndex(0).frame().origin.y
                    + screens.objectAtIndex(0).frame().size.height;
                Ok(screens
                    .iter()
                    .map(|s| {
                        let f = s.visibleFrame();
                        rustcast_core::Frame {
                            x: f.origin.x,
                            y: top - f.origin.y - f.size.height,
                            width: f.size.width,
                            height: f.size.height,
                        }
                    })
                    .collect())
            },
            |effect, ticket| {
                if fixture {
                    ticket.check()
                } else {
                    crate::system_actions::perform(effect, ticket)
                }
            },
        );
    }
    pub fn drain_with(
        &self,
        foreground: impl FnMut() -> Result<Option<Application>, String>,
        activate: impl FnMut(Identity, &Ticket) -> Result<(), String>,
    ) {
        self.drain_with_screens(foreground, activate, || {
            Err("No owned screen fixture supplied.".into())
        });
    }
    pub fn drain_with_screens(
        &self,
        foreground: impl FnMut() -> Result<Option<Application>, String>,
        activate: impl FnMut(Identity, &Ticket) -> Result<(), String>,
        screens: impl FnMut() -> Result<Vec<rustcast_core::Frame>, String>,
    ) {
        self.drain_complete(foreground, activate, screens, |_, _| {
            Err("Complete owned effect fixture unavailable.".into())
        });
    }
    pub fn drain_complete(
        &self,
        mut foreground: impl FnMut() -> Result<Option<Application>, String>,
        mut activate: impl FnMut(Identity, &Ticket) -> Result<(), String>,
        mut screens: impl FnMut() -> Result<Vec<rustcast_core::Frame>, String>,
        mut effect: impl FnMut(crate::system_actions::Effect, &Ticket) -> Result<(), String>,
    ) {
        for _ in 0..8 {
            let Ok(r) = self.receive.try_recv() else {
                break;
            };
            if r.ticket.check().is_err() {
                continue;
            }
            #[cfg(any(test, feature = "test-tools"))]
            {
                if !self.fixture_guard() {
                    let _ = r
                        .reply
                        .try_send(Err("Owned native fixture was not initialized.".into()));
                    continue;
                }
                if self.fixture.load(Ordering::Acquire) {
                    let label = match &r.op {
                        Operation::Foreground => "foreground",
                        Operation::Screens => "screens",
                        Operation::Activate(_) => "activate",
                        Operation::Effect(crate::system_actions::Effect::ActivityMonitor) => {
                            "activity-monitor"
                        }
                        Operation::Effect(crate::system_actions::Effect::Configuration(_)) => {
                            "configuration"
                        }
                        Operation::Effect(crate::system_actions::Effect::Lock) => "lock",
                    };
                    *self
                        .audit
                        .lock()
                        .unwrap_or_else(|p| p.into_inner())
                        .requests
                        .entry(label)
                        .or_default() += 1;
                }
            }
            let reply = match r.op {
                Operation::Effect(value) => effect(value, &r.ticket).map(|_| Reply::Activated),
                Operation::Foreground => foreground().map(Reply::Foreground),
                Operation::Activate(id) => activate(id, &r.ticket).map(|_| Reply::Activated),
                Operation::Screens => screens().map(Reply::Screens),
            };
            if r.ticket.check().is_ok() {
                let _ = r.reply.try_send(reply);
            }
        }
    }
}
#[cfg(test)]
mod fixture_guard_tests {
    use super::*;
    #[test]
    fn missing_startup_fixture_cannot_enter_any_sdk_closure() {
        let (client, main) = channel();
        main.require_fixture(true);
        main.remember();
        let worker = std::thread::spawn(move || {
            let ticket = Ticket {
                deadline: std::time::Instant::now() + std::time::Duration::from_secs(2),
                live: Arc::new(|| true),
            };
            assert!(client.foreground(&ticket).is_err());
            assert!(client.screens(&ticket).is_err());
            for effect in [
                crate::system_actions::Effect::ActivityMonitor,
                crate::system_actions::Effect::Configuration("/owned/config.json".into()),
                crate::system_actions::Effect::Lock,
            ] {
                assert!(client.effect(effect, &ticket).is_err());
            }
            assert!(client
                .activate(
                    Identity {
                        pid: 730001,
                        uid: 0,
                        seconds: 1,
                        micros: 0
                    },
                    &ticket
                )
                .is_err());
            client.fixture_audit()
        });
        while !worker.is_finished() {
            main.drain_complete(
                || panic!("Native foreground reached"),
                |_, _| panic!("Native activation reached"),
                || panic!("Native screens reached"),
                |_, _| panic!("Native effect reached"),
            );
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        let audit = worker.join().unwrap();
        assert_eq!(audit["blocked"], 7);
        assert_eq!(audit["enabled"], false);
        assert_eq!(audit["requests"], serde_json::json!({}));
    }
}
