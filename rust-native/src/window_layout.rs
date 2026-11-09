//! A bounded, verified window-layout owner shared by native and CLI commands.
//! Original AX handles/process start identities are retained for Restore.
use crate::{
    system_search::{Element, Identity, Platform, Ticket, EXPIRED},
    window_placement::{self, Transport},
};
use rustcast_core::Frame;

type Result<T> = std::result::Result<T, String>;
struct Saved {
    identity: Identity,
    window: Element,
    frame: Frame,
}
#[derive(Default)]
pub struct Manager {
    saved: Vec<Saved>,
}
impl Manager {
    pub fn move_window(
        &mut self,
        layout: &str,
        identity: Identity,
        screens: &[Frame],
        platform: &dyn Platform,
        ticket: &Ticket,
    ) -> Result<Frame> {
        ticket.check()?;
        if !platform.trusted() {
            return Err("Allow Ruycast in System Settings → Privacy & Security → Accessibility, then retry.".into());
        }
        if identity.uid != unsafe { libc::getuid() }
            || identity.pid <= 1
            || identity.pid == unsafe { libc::getpid() }
            || platform.identity(identity.pid) != Some(identity)
        {
            return Err("Select a window first.".into());
        }
        let app = platform.application(identity.pid, ticket)?;
        let window = platform
            .element(app, "AXFocusedWindow", ticket)?
            .ok_or("Select a window first.")?;
        let mut io = Placement {
            platform,
            identity,
            app,
            window,
            ticket,
        };
        io.check()?;
        let old = io.read()?;
        let existing = self
            .saved
            .iter()
            .position(|entry| entry.identity == identity && platform.same(entry.window, window));
        let target = if layout == "restore" {
            existing
                .map(|index| self.saved[index].frame)
                .ok_or("No saved frame for this window yet.")?
        } else {
            window_placement::destination(old, screens, layout)?
        };
        if layout != "restore" && existing.is_none() {
            if self.saved.len() > 500 {
                self.saved.clear();
            }
            self.saved.push(Saved {
                identity,
                window,
                frame: old,
            });
        }
        let actual = window_placement::apply(target, &mut io)?;
        if layout == "restore" {
            self.saved.retain(|entry| {
                !(entry.identity == identity && platform.same(entry.window, window))
            });
        }
        Ok(actual)
    }
}
struct Placement<'a> {
    platform: &'a dyn Platform,
    identity: Identity,
    app: Element,
    window: Element,
    ticket: &'a Ticket,
}
impl Transport for Placement<'_> {
    fn check(&mut self) -> Result<()> {
        self.ticket.check()?;
        if !self.platform.trusted()
            || self.platform.identity(self.identity.pid) != Some(self.identity)
            || self.platform.pid(self.app) != Some(self.identity.pid)
            || self.platform.pid(self.window) != Some(self.identity.pid)
            || self
                .platform
                .string(self.window, "AXRole", self.ticket)?
                .as_deref()
                != Some("AXWindow")
            || !self
                .platform
                .elements(self.app, "AXWindows", 50, false, self.ticket)?
                .iter()
                .any(|node| self.platform.same(*node, self.window))
            || !self
                .platform
                .element(self.app, "AXFocusedWindow", self.ticket)?
                .is_some_and(|node| self.platform.same(node, self.window))
        {
            return Err(EXPIRED.into());
        }
        self.ticket.check()
    }
    fn read(&mut self) -> Result<Frame> {
        self.check()?;
        self.platform.frame(self.window, self.ticket)
    }
    fn resize(&mut self, frame: Frame) -> Result<()> {
        self.check()?;
        self.platform.resize(self.window, frame, self.ticket)
    }
    fn position(&mut self, frame: Frame) -> Result<()> {
        self.check()?;
        self.platform.position(self.window, frame, self.ticket)
    }
    fn pause(&mut self, seconds: f64) -> Result<()> {
        self.check()?;
        std::thread::sleep(std::time::Duration::from_secs_f64(
            seconds.min(self.ticket.remaining()?),
        ));
        self.check()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::system_search::fixture::Fixture;
    use std::{
        sync::Arc,
        time::{Duration, Instant},
    };
    #[test]
    fn actual_fixture_transport_moves_restores_and_rejects_stale_identity_or_membership() {
        let fixture = Fixture::default();
        let identity = fixture.previous().unwrap().identity;
        let ticket = Ticket {
            deadline: Instant::now() + Duration::from_secs(2),
            live: Arc::new(|| true),
        };
        let screens = [Frame {
            x: 0.,
            y: 0.,
            width: 1000.,
            height: 800.,
        }];
        let old = fixture.frame(Element(2), &ticket).unwrap();
        let mut manager = Manager::default();
        let placed = manager
            .move_window("left-half", identity, &screens, &fixture, &ticket)
            .unwrap();
        assert!(window_placement::matches(
            placed,
            Frame {
                x: 0.,
                y: 0.,
                width: 500.,
                height: 800.
            }
        ));
        let restored = manager
            .move_window("restore", identity, &screens, &fixture, &ticket)
            .unwrap();
        assert!(window_placement::matches(old, restored));
        assert!(manager
            .move_window("restore", identity, &screens, &fixture, &ticket)
            .is_err());
        let before = fixture.effects.lock().unwrap().len();
        fixture.edit(Element(1), |n| {
            n.lists.insert("AXWindows".into(), vec![]);
        });
        assert!(manager
            .move_window("maximize", identity, &screens, &fixture, &ticket)
            .is_err());
        assert_eq!(fixture.effects.lock().unwrap().len(), before);
        let mut stale = identity;
        stale.seconds += 1;
        assert!(manager
            .move_window("maximize", stale, &screens, &fixture, &ticket)
            .is_err());
    }
}
