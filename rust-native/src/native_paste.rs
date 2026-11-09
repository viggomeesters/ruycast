//! Main-owned queued snippet effects. No native handle crosses a worker.
use crate::main_loop::Timer;
use objc2::{rc::Retained, MainThreadMarker};
use objc2_app_kit::{
    NSApplicationActivationOptions, NSPasteboard, NSRunningApplication, NSWorkspace,
};
use objc2_foundation::NSObjectProtocol;
use ruycast_native::{
    paste::{self, Outcome, Ticket},
    profile::ProfilePaths,
    snippet_clipboard::{PreparedItem, Publish},
};
use std::{
    collections::VecDeque,
    ffi::c_void,
    rc::Rc,
    sync::Arc,
    time::{Duration, Instant},
};

#[link(name = "ApplicationServices", kind = "framework")]
extern "C" {
    fn AXIsProcessTrusted() -> u8;
    fn CGEventCreateKeyboardEvent(source: *const c_void, key: u16, down: bool) -> *mut c_void;
    fn CGEventSetFlags(event: *mut c_void, flags: u64);
    fn CGEventSetIntegerValueField(event: *mut c_void, field: u32, value: i64);
    fn CGEventPost(tap: u32, event: *mut c_void);
}
#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {
    fn CFRelease(object: *const c_void);
}
const GENERATED_TAG: i64 = 0x52554859504552;
enum Phase {
    Commit,
    Hide,
    Paste(Instant),
    Cursor { at: Instant, remaining: usize },
}
enum Step {
    Pending(Pending),
    Finished(Ticket, Outcome),
}
struct Pending {
    ticket: Ticket,
    app: Option<Retained<NSRunningApplication>>,
    item: Option<PreparedItem>,
    phase: Phase,
    copied: isize,
}
pub struct Service {
    main: MainThreadMarker,
    headless: bool,
    fixture: bool,
    #[cfg(feature = "test-tools")]
    home: std::path::PathBuf,
    previous: Option<Retained<NSRunningApplication>>,
    board: Option<Retained<NSPasteboard>>,
    own_write: Rc<ruycast_native::clipboard_ownership::OwnWrites>,
    queue: VecDeque<Ticket>,
    active: Option<Pending>,
    timer: Option<Timer>,
    wake: Arc<dyn Fn() + Send + Sync>,
}
impl Service {
    pub fn new(
        main: MainThreadMarker,
        paths: &ProfilePaths,
        headless: bool,
        fixture: bool,
        wake: Arc<dyn Fn() + Send + Sync>,
        own_write: Rc<ruycast_native::clipboard_ownership::OwnWrites>,
    ) -> Self {
        #[cfg(not(feature = "test-tools"))]
        let _ = paths;
        Self {
            main,
            headless,
            fixture,
            #[cfg(feature = "test-tools")]
            home: paths.home.clone(),
            previous: None,
            board: None,
            own_write,
            queue: VecDeque::new(),
            active: None,
            timer: None,
            wake,
        }
    }
    fn boundary(&self) -> bool {
        if !self.fixture {
            return true;
        }
        #[cfg(feature = "test-tools")]
        unsafe {
            if !ruycast_native::workspace::fixture_installed(&self.home) {
                return false;
            }
            let ready = libc::dlsym(
                libc::RTLD_DEFAULT,
                c"ruycast_snippet_fixture_ready".as_ptr(),
            );
            let names = [
                c"ruycast_snippet_fixture_trusted",
                c"ruycast_snippet_fixture_create",
                c"ruycast_snippet_fixture_flags",
                c"ruycast_snippet_fixture_integer",
                c"ruycast_snippet_fixture_post",
            ];
            if ready.is_null()
                || names
                    .iter()
                    .any(|name| libc::dlsym(libc::RTLD_DEFAULT, name.as_ptr()).is_null())
            {
                return false;
            }
            let ready: unsafe extern "C" fn() -> u64 = std::mem::transmute(ready);
            ready() == 0x5255504153544531
        }
        #[cfg(not(feature = "test-tools"))]
        false
    }
    pub fn remember_target(&mut self) {
        if self.headless || !self.boundary() {
            return;
        }
        if let Some(app) = NSWorkspace::sharedWorkspace().frontmostApplication() {
            let pid = app.processIdentifier();
            if pid > 0 && pid != unsafe { libc::getpid() } {
                self.previous = Some(app);
            }
        }
    }
    fn board(&mut self) -> Retained<NSPasteboard> {
        self.board
            .get_or_insert_with(|| {
                if self.fixture {
                    #[cfg(feature="test-tools")]
                    if let Some(board)=crate::native_services::shared_fixture_board(){return board}
                    NSPasteboard::pasteboardWithUniqueName()
                } else {
                    NSPasteboard::generalPasteboard()
                }
            })
            .clone()
    }
    fn begin(&mut self, ticket: Ticket) -> Step {
        let fail = |ticket, error| Step::Finished(ticket, Outcome::failure(error, false));
        if !ticket.authority.live() {
            let outcome = ticket.authority.dispatch.retire();
            return Step::Finished(ticket, outcome);
        }
        if !self.boundary() {
            return fail(
                ticket,
                "Complete snippet fixture boundary is not installed.",
            );
        }
        if ticket.prepared.copy_only {
            let item=if ticket.prepared.concealed {PreparedItem::concealed(self.main,&ticket.prepared.payload.text)}else if let Some(payload)=&ticket.prepared.clipboard{PreparedItem::history(self.main,payload)}else{PreparedItem::new(self.main,&ticket.prepared.payload)};
            let Some(item)=item else{return fail(ticket,"Clipboard item could not be prepared.")};
            if !ticket.request_grant(false){let outcome=ticket.authority.dispatch.retire();return Step::Finished(ticket,outcome)}
            return Step::Pending(Pending{ticket,app:None,item:Some(item),phase:Phase::Commit,copied:0});
        }
        let trusted = if self.fixture {
            #[cfg(feature = "test-tools")]
            unsafe {
                let f: unsafe extern "C" fn() -> u8 = std::mem::transmute(libc::dlsym(
                    libc::RTLD_DEFAULT,
                    c"ruycast_snippet_fixture_trusted".as_ptr(),
                ));
                f() != 0
            }
            #[cfg(not(feature = "test-tools"))]
            false
        } else {
            unsafe { AXIsProcessTrusted() != 0 }
        };
        if !trusted {
            return fail(ticket,"Allow Ruycast in System Settings → Privacy & Security → Accessibility to insert text.");
        }
        self.remember_target();
        let Some(remembered) = self.previous.as_ref().filter(|_| !self.headless) else {
            return fail(ticket, "Choose an app to insert the snippet into.");
        };
        let Some(app) = NSRunningApplication::runningApplicationWithProcessIdentifier(
            remembered.processIdentifier(),
        ) else {
            return fail(ticket, "Choose an app to insert the snippet into.");
        };
        if app.isTerminated() || !app.isEqual(Some(remembered)) {
            return fail(ticket, "Choose an app to insert the snippet into.");
        }
        let item=if ticket.prepared.concealed {PreparedItem::concealed(self.main,&ticket.prepared.payload.text)}else if let Some(payload)=&ticket.prepared.clipboard {PreparedItem::history(self.main,payload)}else{PreparedItem::new(self.main,&ticket.prepared.payload)};
        let Some(item) = item else {
            return fail(ticket, "Could not copy text.");
        };
        if !ticket.authority.live() {
            let outcome = ticket.authority.dispatch.retire();
            return Step::Finished(ticket, outcome);
        }
        if !ticket.request_grant(false) {
            let outcome = ticket.authority.dispatch.retire();
            return Step::Finished(ticket, outcome);
        }
        Step::Pending(Pending {
            ticket,
            app:Some(app),
            item: Some(item),
            phase: Phase::Commit,
            copied: 0,
        })
    }
    fn target(&self, expected: &NSRunningApplication) -> bool {
        NSWorkspace::sharedWorkspace()
            .frontmostApplication()
            .is_some_and(|app| !expected.isTerminated() && app.isEqual(Some(expected)))
    }
    fn pair(&self, key: u16, flags: u64, allowed: impl Fn() -> bool) -> Result<(), String> {
        type Create = unsafe extern "C" fn(*const c_void, u16, bool) -> *mut c_void;
        type Flags = unsafe extern "C" fn(*mut c_void, u64);
        type Integer = unsafe extern "C" fn(*mut c_void, u32, i64);
        type Post = unsafe extern "C" fn(u32, *mut c_void);
        #[allow(unused_mut)]
        let mut functions: (Create, Flags, Integer, Post) = (
            CGEventCreateKeyboardEvent,
            CGEventSetFlags,
            CGEventSetIntegerValueField,
            CGEventPost,
        );
        if self.fixture && !self.boundary() {
            return Err("Complete snippet fixture boundary is not installed.".into());
        }
        #[cfg(feature = "test-tools")]
        if self.fixture {
            unsafe {
                // Every fixture entry is mandatory; no fallback to real HID.
                functions = (
                    std::mem::transmute::<*mut c_void, Create>(libc::dlsym(
                        libc::RTLD_DEFAULT,
                        c"ruycast_snippet_fixture_create".as_ptr(),
                    )),
                    std::mem::transmute::<*mut c_void, Flags>(libc::dlsym(
                        libc::RTLD_DEFAULT,
                        c"ruycast_snippet_fixture_flags".as_ptr(),
                    )),
                    std::mem::transmute::<*mut c_void, Integer>(libc::dlsym(
                        libc::RTLD_DEFAULT,
                        c"ruycast_snippet_fixture_integer".as_ptr(),
                    )),
                    std::mem::transmute::<*mut c_void, Post>(libc::dlsym(
                        libc::RTLD_DEFAULT,
                        c"ruycast_snippet_fixture_post".as_ptr(),
                    )),
                );
            }
        }
        struct Event(*mut c_void);
        impl Drop for Event {
            fn drop(&mut self) {
                if !self.0.is_null() {
                    unsafe { CFRelease(self.0) }
                }
            }
        }
        let down = Event(unsafe { (functions.0)(std::ptr::null(), key, true) });
        let up = Event(unsafe { (functions.0)(std::ptr::null(), key, false) });
        if down.0.is_null() || up.0.is_null() {
            return Err("Could not prepare snippet input events.".into());
        }
        for event in [&down, &up] {
            unsafe {
                (functions.1)(event.0, flags);
                (functions.2)(event.0, 42, GENERATED_TAG);
            }
        }
        if !allowed() {
            return Err("Snippet target or request changed before posting input.".into());
        }
        // Once down is posted, always release its matching key even if a
        // concurrent retirement occurs; never leave a synthetic key held.
        unsafe {
            (functions.3)(0, down.0);
            (functions.3)(0, up.0);
        }
        Ok(())
    }
    fn advance(
        &mut self,
        mut pending: Pending,
        hide: &mut impl FnMut(&ruycast_native::connected::HideToken) -> Result<(), String>,
    ) -> Step {
        if !pending.ticket.authority.live() {
            let outcome = pending.ticket.authority.dispatch.retire();
            return Step::Finished(pending.ticket, outcome);
        }
        let failure = |ticket: Ticket, error: String| {
            let committed = ticket.authority.dispatch.committed();
            Step::Finished(ticket, Outcome::failure(error, committed))
        };
        match pending.phase {
            Phase::Commit => {
                let Some(grant) = pending.ticket.authority.take_grant() else {
                    return Step::Pending(pending);
                };
                if let Err(error) = grant {
                    return failure(pending.ticket, error);
                }
                if !pending.ticket.authority.live() {
                    let outcome = pending.ticket.authority.dispatch.retire();
                    return Step::Finished(pending.ticket, outcome);
                }
                // The owner grant is asynchronous. Revalidate the retained
                // process incarnation immediately before potential mutation.
                if let Some(app)=pending.app.as_ref(){
                let current = NSRunningApplication::runningApplicationWithProcessIdentifier(
                    app.processIdentifier(),
                );
                if app.isTerminated()
                    || current
                        .as_ref()
                        .is_none_or(|current| !current.isEqual(Some(app)))
                {
                    return failure(
                        pending.ticket,
                        "Choose an app to insert the snippet into.".into(),
                    );
                }
                }
                let board = self.board();
                let item = pending.item.take().expect("prepared item before grant");
                let Some(_own_guard) = self.own_write.begin() else {
                    return failure(pending.ticket, "Clipboard write is busy.".into());
                };
                let expected = item.clone();
                match item.publish(&board, || {
                    pending.ticket.authority.live() && pending.ticket.authority.dispatch.enter()
                }) {
                    Publish::Cancelled => {
                        let outcome = pending.ticket.authority.dispatch.retire();
                        return Step::Finished(pending.ticket, outcome);
                    }
                    Publish::Attempted(false) => {
                        let count = board.changeCount();
                        if expected.observes_own_content(&board) && board.changeCount() == count {
                            self.own_write.set(Some(count));
                        }
                        return failure(pending.ticket,"Could not copy text. The clipboard may have changed; check it before trying again.".into());
                    }
                    Publish::Attempted(true) => {}
                }
                pending.copied = board.changeCount();
                if !expected.matches(&board) || board.changeCount() != pending.copied {
                    return failure(
                        pending.ticket,
                        "The clipboard changed before the snippet could be pasted.".into(),
                    );
                }
                self.own_write.set(Some(pending.copied));
                if pending.ticket.prepared.copy_only {
                    return Step::Finished(pending.ticket,Outcome{response:serde_json::json!({"ok":true,"result":{"state":"copied"}}),committed:true});
                }
                if !pending.ticket.request_grant(true) {
                    let outcome = pending.ticket.authority.dispatch.retire();
                    return Step::Finished(pending.ticket, outcome);
                }
                pending.phase = Phase::Hide;
            }
            Phase::Hide => {
                let Some(grant) = pending.ticket.authority.take_grant() else {
                    return Step::Pending(pending);
                };
                if let Err(error) = grant {
                    return failure(pending.ticket, error);
                }
                let token = pending
                    .ticket
                    .hide
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .clone();
                let Some(token) = token else {
                    return failure(
                        pending.ticket,
                        "Snippet window owner is unavailable after copying.".into(),
                    );
                };
                if let Err(error) = hide(&token) {
                    return failure(pending.ticket, error);
                }
                if !pending.ticket.authority.live() {
                    let outcome = pending.ticket.authority.dispatch.retire();
                    return Step::Finished(pending.ticket, outcome);
                }
                if self.board().changeCount() != pending.copied {
                    return failure(
                        pending.ticket,
                        "The clipboard changed before the snippet could be pasted.".into(),
                    );
                }
                let _ = pending
                    .app.as_ref().expect("paste target")
                    .activateWithOptions(NSApplicationActivationOptions::empty());
                pending.phase = Phase::Paste(Instant::now() + Duration::from_millis(150));
            }
            Phase::Paste(at) => {
                if Instant::now() < at {
                    return Step::Pending(pending);
                }
                if self.board().changeCount() != pending.copied {
                    return failure(
                        pending.ticket,
                        "The clipboard changed before the snippet could be pasted.".into(),
                    );
                }
                if !self.target(pending.app.as_ref().expect("paste target")) {
                    return failure(
                        pending.ticket,
                        "Copied · target app did not activate".into(),
                    );
                }
                let allowed = || {
                    pending.ticket.authority.live()
                        && self.target(pending.app.as_ref().expect("paste target"))
                        && self
                            .board
                            .as_ref()
                            .is_some_and(|b| b.changeCount() == pending.copied)
                };
                if let Err(error) = self.pair(9, 1 << 20, allowed) {
                    return failure(pending.ticket, error);
                }
                if let Some(remaining) = pending.ticket.prepared.cursor_steps {
                    pending.phase = Phase::Cursor {
                        at: Instant::now() + Duration::from_millis(80),
                        remaining,
                    };
                } else {
                    return Step::Finished(pending.ticket, Outcome::success());
                }
            }
            Phase::Cursor { at, remaining } => {
                if Instant::now() < at {
                    return Step::Pending(pending);
                }
                if !self.target(pending.app.as_ref().expect("paste target")) {
                    return failure(
                        pending.ticket,
                        "Snippet target changed before cursor movement.".into(),
                    );
                }
                for _ in 0..remaining.min(64) {
                    if let Err(error) = self.pair(123, 0, || {
                        pending.ticket.authority.live() && self.target(pending.app.as_ref().expect("paste target"))
                    }) {
                        return failure(pending.ticket, error);
                    }
                }
                if remaining <= 64 {
                    return Step::Finished(pending.ticket, Outcome::success());
                }
                pending.phase = Phase::Cursor {
                    at: Instant::now(),
                    remaining: remaining - 64,
                };
            }
        }
        Step::Pending(pending)
    }
    pub fn pump(
        &mut self,
        tickets: Vec<Ticket>,
        mut hide: impl FnMut(&ruycast_native::connected::HideToken) -> Result<(), String>,
    ) -> Result<(), String> {
        self.timer.take();
        let old = std::mem::take(&mut self.queue);
        for ticket in old {
            if ticket.authority.live() {
                self.queue.push_back(ticket);
            } else {
                let outcome = ticket.authority.dispatch.retire();
                ticket.finish(outcome);
            }
        }
        for ticket in tickets {
            if self.queue.len() + usize::from(self.active.is_some()) >= paste::LIMIT {
                ticket.finish(Outcome::failure(
                    "Snippet paste is busy. Try again after an active command finishes.",
                    false,
                ));
            } else {
                self.queue.push_back(ticket);
            }
        }
        // Process finite state transitions; timers/grants yield to native UI.
        for _ in 0..paste::LIMIT {
            if self.active.is_none() {
                let Some(ticket) = self.queue.pop_front() else {
                    break;
                };
                match self.begin(ticket) {
                    Step::Pending(pending) => self.active = Some(pending),
                    Step::Finished(ticket, outcome) => {
                        ticket.finish(outcome);
                        continue;
                    }
                }
            }
            let pending = self.active.take().expect("active paste");
            match self.advance(pending, &mut hide) {
                Step::Pending(pending) => {
                    self.active = Some(pending);
                    break;
                }
                Step::Finished(ticket, outcome) => ticket.finish(outcome),
            }
        }
        if let Some(pending) = &self.active {
            let phase_due = match pending.phase {
                Phase::Paste(at) | Phase::Cursor { at, .. } => Some(at),
                _ => None,
            };
            let due = phase_due
                .into_iter()
                .chain(pending.ticket.authority.deadline)
                .min();
            if let Some(due) = due {
                let wake = self.wake.clone();
                self.timer = Some(Timer::new_default(
                    self.main,
                    due.saturating_duration_since(Instant::now()).as_secs_f64(),
                    move || wake(),
                )?);
            }
        } else if !self.queue.is_empty() {
            (self.wake)();
        }
        self.record_state();
        Ok(())
    }
    fn record_state(&self) {
        #[cfg(feature = "test-tools")]
        if self.fixture && self.boundary() {
            let phase = self.active.as_ref().map(|p| match p.phase {
                Phase::Commit => "commit-grant",
                Phase::Hide => "hide-grant",
                Phase::Paste(_) => "paste-timer",
                Phase::Cursor { .. } => "cursor-timer",
            });
            let value = serde_json::json!({"active":self.active.is_some(),"pending":self.queue.len(),"phase":phase,"timer":self.timer.is_some(),"previousPID":self.previous.as_ref().map(|a|a.processIdentifier()),"ownChangeCount":self.own_write.get()});
            let tmp = self.home.join(".snippet-status.tmp");
            if std::fs::write(&tmp, value.to_string()).is_ok() {
                let _ = std::fs::rename(tmp, self.home.join("snippet-status.json"));
            }
        }
    }
    pub fn stop(&mut self) {
        self.timer.take();
        if let Some(pending) = self.active.take() {
            let outcome = pending.ticket.authority.dispatch.retire();
            pending.ticket.finish(outcome);
        }
        for ticket in self.queue.drain(..) {
            let outcome = ticket.authority.dispatch.retire();
            ticket.finish(outcome);
        }
        self.record_state();
    }
}
impl Drop for Service {
    fn drop(&mut self) {
        self.stop();
        if self.fixture {
            if let Some(board) = self.board.take() {
                board.clearContents();
                unsafe {
                    let _: () = objc2::msg_send![&*board, releaseGlobally];
                }
            }
        }
    }
}
